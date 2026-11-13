use std::future::poll_fn;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio_postgres::{AsyncMessage, NoTls};
use tracing::instrument;
use uuid::Uuid;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{
    AnalyticsRepository, CurrencySeries, DailyPoint, DailySeries, JobQueueRepository, PendingFact,
    QueuedJob, ReconcileMismatch, Result, SeriesPoint, SpendPurchase, SpendRange,
    StuckOutboxItem, TrendBucket, UnresolvedPurchase, MAX_ATTEMPTS,
};
use crate::repository::{PgPool, RepositoryError};

/// Rows are claimed with `SKIP LOCKED`; a claim older than this is reclaimable,
/// which also makes a crashed projector recover automatically.
const CLAIM_TIMEOUT_MINUTES: i32 = 5;
/// Channel the write path signals after queueing outbox work.
pub const OUTBOX_CHANNEL: &str = "analytics_outbox";

/// Drives a dedicated LISTEN connection and pokes `wake` on every notification.
///
/// This deliberately does not use the bb8 pool: bb8-postgres drives the
/// `Connection` future itself and drops `Notification` messages, and
/// tokio-postgres only surfaces notifications through
/// `Connection::poll_message`. So unlike a pooled connection, this one is never
/// reaped or recycled by bb8 and its reconnect handling is ours.
///
/// Notifications are *not* queued for absent listeners, and a half-open socket
/// can swallow them, so callers must keep a polling fallback.
#[tracing::instrument(skip_all)]
pub async fn listen_forever(config: tokio_postgres::Config, wake: Arc<Notify>) {
    let mut backoff = Duration::from_millis(500);

    loop {
        let connected_at = std::time::Instant::now();
        if let Err(e) = listen_once(&config, &wake).await {
            tracing::error!(error = %e, "outbox listener lost its connection");
        }
        tracing::info!(gauge.outbox_listener_connected = 0);
        tracing::info!(monotonic_counter.outbox_listener_reconnects_total = 1u64);

        // A connection that stayed up for a while means the peer accepted us;
        // retry quickly. Otherwise back off (e.g. database still starting).
        if connected_at.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_millis(500);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

async fn listen_once(config: &tokio_postgres::Config, wake: &Arc<Notify>) -> Result<()> {
    let (client, mut connection) = config
        .connect(NoTls)
        .await
        .map_err(|e| ApplicationError::Internal(e.to_string()))?;

    // The connection must be driven for LISTEN to complete and for
    // notifications to arrive; this task does both.
    let notify = wake.clone();
    let driver = tokio::spawn(async move {
        loop {
            match poll_fn(|cx| connection.poll_message(cx)).await {
                Some(Ok(AsyncMessage::Notification(_))) => notify.notify_one(),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(ApplicationError::Internal(e.to_string())),
                None => return Ok(()),
            }
        }
    });

    tokio::time::timeout(
        Duration::from_secs(10),
        client.batch_execute(&format!("LISTEN {OUTBOX_CHANNEL}")),
    )
    .await
    .map_err(|_| ApplicationError::Internal("timed out issuing LISTEN".into()))?
    .map_err(|e| ApplicationError::Internal(e.to_string()))?;

    tracing::info!(channel = OUTBOX_CHANNEL, "outbox listener ready");
    tracing::info!(gauge.outbox_listener_connected = 1);

    match driver.await {
        Ok(result) => result,
        Err(e) => Err(ApplicationError::Internal(format!(
            "listener driver task failed: {e}"
        ))),
    }
}

/// Groups ordered rows into one series per currency. Rows must arrive sorted by
/// currency so each currency is produced once. Shared by every series query, so
/// the grouping rule exists in one place.
fn group_by_currency<P, T>(
    pairs: impl Iterator<Item = (String, P)>,
    build: impl Fn(String, Vec<P>) -> T,
) -> Vec<T> {
    let mut grouped: Vec<(String, Vec<P>)> = Vec::new();
    for (currency, point) in pairs {
        match grouped.last_mut().filter(|(c, _)| *c == currency) {
            Some((_, points)) => points.push(point),
            None => grouped.push((currency, vec![point])),
        }
    }
    grouped.into_iter().map(|(c, pts)| build(c, pts)).collect()
}

#[derive(Clone)]
pub struct PgAnalyticsRepo {
    pool: PgPool,
}

impl PgAnalyticsRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AnalyticsRepository for PgAnalyticsRepo {

    /// Atomically claims up to `limit` pending outbox rows.
    ///
    /// The `UPDATE ... WHERE id IN (SELECT ... FOR UPDATE SKIP LOCKED)` form
    /// takes the row locks and stamps `claimed_at` in one statement, so several
    /// projectors can run without processing the same row.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.claim_outbox"))]
    async fn claim_outbox(&self, limit: i64) -> Result<Vec<PendingFact>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                r#"UPDATE analytics_outbox
                      SET claimed_at = NOW(),
                          attempts   = attempts + 1
                    WHERE id IN (
                        SELECT id
                          FROM analytics_outbox
                         WHERE processed_at IS NULL
                           AND attempts < $2
                           AND (claimed_at IS NULL
                                OR claimed_at < NOW() - make_interval(mins => $3))
                         ORDER BY id
                         LIMIT $1
                         FOR UPDATE SKIP LOCKED
                    )
                    RETURNING id, user_id, aggregate_id"#,
                &[&limit, &MAX_ATTEMPTS, &CLAIM_TIMEOUT_MINUTES],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows
            .iter()
            .map(|r| PendingFact {
                outbox_id: r.get("id"),
                user_id: r.get("user_id"),
                purchase_id: r.get("aggregate_id"),
            })
            .collect())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.mark_processed"))]
    async fn mark_processed(&self, ids: &[i64]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        conn.execute(
            "UPDATE analytics_outbox SET processed_at = NOW(), last_error = NULL WHERE id = ANY($1)",
            &[&ids],
        )
        .await
        .map_err(RepositoryError::DbError)?;
        Ok(())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.mark_failed"))]
    async fn mark_failed(&self, id: i64, error: &str) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        conn.execute(
            r#"UPDATE analytics_outbox
                  SET last_error = $2, claimed_at = NULL
                WHERE id = $1"#,
            &[&id, &error],
        )
        .await
        .map_err(RepositoryError::DbError)?;
        Ok(())
    }

    /// Deletes and re-inserts the fact row for one purchase.
    ///
    /// Delete-then-insert (rather than upsert) because an edited `purchase_date`
    /// changes `occurred_at`, which is part of the primary key. The conversion
    /// reuses `purchase_items.total_wallet`, so the rounding rule lives in one
    /// place. A soft-deleted purchase simply produces no rows.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.rebuild_fact"))]
    async fn rebuild_fact(&self, user_id: Uuid, purchase_id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let tran = conn.transaction().await.map_err(RepositoryError::DbError)?;

        tran.execute(
            "DELETE FROM purchase_fact WHERE purchase_id = $1",
            &[&purchase_id],
        )
        .await
        .map_err(RepositoryError::DbError)?;

        tran.execute(
            r#"INSERT INTO purchase_fact
                   (occurred_at, purchase_id, version, user_id, wallet_id, category_id,
                    merchant_id, currency, amount_minor, exchange_rate_scaled, base_currency,
                    amount_base_minor, resolution)
               SELECT v.purchase_date, v.purchase_id, p.version, v.user_id, v.wallet_id,
                      p.category_id, p.merchant_id, v.source_currency, v.total::bigint,
                      p.exchange_rate_scaled, v.wallet_currency, v.total_wallet,
                      v.resolution
                 FROM purchase_items v
                 JOIN purchase p ON p.id = v.purchase_id
                WHERE v.purchase_id = $1
                  AND p.user_id = $2"#,
            &[&purchase_id, &user_id],
        )
        .await
        .map_err(RepositoryError::DbError)?;

        tran.commit().await.map_err(RepositoryError::DbError)?;
        Ok(())
    }

    /// Enqueues every live purchase. Idempotent at the projection level, and the
    /// outbox already processed for a purchase is harmless (coalesced by id).
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.enqueue_all"))]
    async fn enqueue_all_purchases(&self) -> Result<u64> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let n = conn
            .execute(
                r#"INSERT INTO analytics_outbox
                       (user_id, aggregate_type, aggregate_id, aggregate_version)
                   SELECT user_id, 'purchase', id, version
                     FROM purchase
                    WHERE deleted_at IS NULL"#,
                &[],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(n)
    }

    /// Deletes processed rows older than `cutoff`. Pending rows are never
    /// touched: removing them would lose a Purchase's projection.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.purge_processed"))]
    async fn purge_processed_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let deleted = conn
            .execute(
                "DELETE FROM analytics_outbox
                  WHERE processed_at IS NOT NULL AND processed_at < $1",
                &[&cutoff],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(deleted)
    }

    /// Bucketed totals per currency, in the user's calendar timezone.
    ///
    /// `time_bucket` with a timezone argument is what makes a "day" or "month"
    /// mean the user's day or month rather than UTC's. Totals come from
    /// `amount_base_minor`, which is already denominated in the wallet's currency
    /// at purchase time, so nothing is converted here and nothing is restated.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.bucketed_spend"))]
    async fn bucketed_spend(
        &self,
        user_id: Uuid,
        range: &SpendRange,
        bucket: TrendBucket,
    ) -> Result<Vec<CurrencySeries>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                r#"SELECT base_currency,
                          time_bucket(($2::text)::interval, occurred_at, $3) AS bucket_start,
                          COALESCE(SUM(amount_base_minor), 0)::bigint AS total_minor,
                          COUNT(*) FILTER (WHERE resolution <> 'unresolved') AS purchases,
                          COUNT(*) FILTER (WHERE resolution = 'unresolved') AS unresolved
                     FROM purchase_fact
                    WHERE user_id = $1
                      AND occurred_at >= $4
                      AND occurred_at < $5
                      AND ($6::uuid IS NULL OR wallet_id = $6)
                      AND ($7::integer IS NULL OR category_id = $7)
                      AND ($8::text IS NULL OR base_currency = $8)
                    GROUP BY 1, 2
                    ORDER BY 1, 2"#,
                &[
                    &user_id,
                    &bucket.as_interval(),
                    &range.timezone,
                    &range.from,
                    &range.to,
                    &range.wallet_id,
                    &range.category_id,
                    &range.currency,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(group_by_currency(
            rows.iter().map(|r| {
                (
                    r.get::<_, String>("base_currency"),
                    SeriesPoint {
                        bucket_start: r.get("bucket_start"),
                        total_minor: r.get("total_minor"),
                        purchases: r.get("purchases"),
                        unresolved: r.get("unresolved"),
                    },
                )
            }),
            |currency, points| CurrencySeries { currency, points },
        ))
    }

    /// Daily totals per currency.
    ///
    /// The rolling average is computed in the service from these points: a
    /// window frame in SQL would need the window size as a parameter, and this
    /// keeps all money rounding in one language with one rule.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.daily_spend"))]
    async fn daily_spend(
        &self,
        user_id: Uuid,
        range: &SpendRange,
        _window_days: i32,
    ) -> Result<Vec<DailySeries>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                // The day label is the bucket start *rendered in the user's zone*.
                // Casting `time_bucket(..., tz)` straight to `date` would label a
                // 22:00Z Madrid bucket as the previous day, because the cast uses
                // the session timezone. Gaps are filled from a calendar series so a
                // trailing window averages calendar days, not merely days with spend.
                r#"WITH spent AS (
                       SELECT base_currency,
                              time_bucket('1 day', occurred_at, $2) AS day_start,
                              SUM(amount_base_minor) AS total_minor
                         FROM purchase_fact
                        WHERE user_id = $1
                          AND occurred_at >= $3
                          AND occurred_at < $4
                          AND ($5::uuid IS NULL OR wallet_id = $5)
                          AND ($6::integer IS NULL OR category_id = $6)
                          AND ($7::text IS NULL OR base_currency = $7)
                        GROUP BY 1, 2
                   ), days AS (
                       SELECT DISTINCT base_currency,
                              (day_start AT TIME ZONE $2)::date AS day
                         FROM spent
                   ), span AS (
                       -- The span runs the whole requested range, not just the
                       -- days with spend: an "average daily spend over N days"
                       -- must count empty days as zero, both before the first
                       -- purchase and after the last one. Stopping at
                       -- MAX(day) dropped the trailing empties, so a user who
                       -- stopped spending last week had a pace measured over
                       -- fewer days than the window -- an overstated pace.
                       SELECT DISTINCT d.base_currency,
                              generate_series(
                                  ($3 AT TIME ZONE $2)::date,
                                  (($4 - INTERVAL '1 day') AT TIME ZONE $2)::date,
                                  INTERVAL '1 day'
                              )::date AS day
                         FROM days d
                   )
                   SELECT span.base_currency,
                          span.day,
                          COALESCE(SUM(s.total_minor), 0)::bigint AS total_minor
                     FROM span
                     LEFT JOIN spent s
                       ON s.base_currency = span.base_currency
                      AND (s.day_start AT TIME ZONE $2)::date = span.day
                    GROUP BY 1, 2
                    ORDER BY 1, 2"#,
                &[
                    &user_id,
                    &range.timezone,
                    &range.from,
                    &range.to,
                    &range.wallet_id,
                    &range.category_id,
                    &range.currency,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(group_by_currency(
            rows.iter().map(|r| {
                (
                    r.get::<_, String>("base_currency"),
                    DailyPoint {
                        day: r.get("day"),
                        total_minor: r.get("total_minor"),
                    },
                )
            }),
            |currency, points| DailySeries { currency, points },
        ))
    }

    /// Purchases recorded with an unresolved conversion. Uses the partial index
    /// over `resolution = 'unresolved'`.
    async fn unresolved_purchases(&self) -> Result<Vec<UnresolvedPurchase>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                "SELECT purchase_id, user_id, currency, base_currency,
                        amount_minor, occurred_at
                   FROM purchase_fact
                  WHERE resolution = 'unresolved'
                  ORDER BY occurred_at",
                &[],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(rows
            .iter()
            .map(|r| UnresolvedPurchase {
                purchase_id: r.get("purchase_id"),
                user_id: r.get("user_id"),
                source_currency: r.get("currency"),
                wallet_currency: r.get("base_currency"),
                amount_minor: r.get("amount_minor"),
                occurred_at: r.get("occurred_at"),
            })
            .collect())
    }

    /// Rows that exhausted their retries: unprocessed and past the cap.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.stuck_outbox"))]
    async fn stuck_outbox(&self) -> Result<Vec<StuckOutboxItem>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                "SELECT id, user_id, aggregate_id, attempts, last_error
                   FROM analytics_outbox
                  WHERE processed_at IS NULL AND attempts >= $1
                  ORDER BY id",
                &[&MAX_ATTEMPTS],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(rows
            .iter()
            .map(|r| StuckOutboxItem {
                outbox_id: r.get("id"),
                user_id: r.get("user_id"),
                purchase_id: r.get("aggregate_id"),
                attempts: r.get("attempts"),
                last_error: r.get("last_error"),
            })
            .collect())
    }

    /// Age of the oldest *pending* outbox row, in seconds.
    ///
    /// Rows that exhausted their retries are excluded: they are not pending
    /// work, and counting them would turn this reading into a floor that never
    /// clears. They are surfaced by `stuck_outbox` instead.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.outbox_lag"))]
    async fn outbox_lag_seconds(&self) -> Result<Option<f64>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_one(
                r#"SELECT EXTRACT(EPOCH FROM (NOW() - MIN(created_at)))::float8 AS lag
                     FROM analytics_outbox
                    WHERE processed_at IS NULL
                      AND attempts < $1"#,
                &[&MAX_ATTEMPTS],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(row.get("lag"))
    }

    /// The recorded purchases in a range, so the burn rate can tell which of
    /// them are the payments of a declared recurring rule.
    ///
    /// The amount is the purchase's own amount in its purchase currency, not
    /// the converted one: a subscription is matched on what was charged.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.purchases_in_range"))]
    async fn purchases_in_range(
        &self,
        user_id: Uuid,
        range: &SpendRange,
    ) -> Result<Vec<SpendPurchase>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let rows = conn
            .query(
                r#"SELECT purchase_id, merchant_id, category_id, amount_minor, currency, occurred_at
                     FROM purchase_fact
                    WHERE user_id = $1
                      AND occurred_at >= $2
                      AND occurred_at < $3
                      AND ($4::uuid IS NULL OR wallet_id = $4)
                      AND ($5::integer IS NULL OR category_id = $5)
                      AND ($6::text IS NULL OR base_currency = $6)
                      AND resolution <> 'unresolved'
                    ORDER BY occurred_at"#,
                &[
                    &user_id,
                    &range.from,
                    &range.to,
                    &range.wallet_id,
                    &range.category_id,
                    &range.currency,
                ],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows
            .iter()
            .map(|row| SpendPurchase {
                purchase_id: row.get("purchase_id"),
                merchant_id: row.get("merchant_id"),
                category_id: row.get("category_id"),
                amount_minor: row.get("amount_minor"),
                currency: row.get("currency"),
                occurred_at: row.get("occurred_at"),
            })
            .collect())
    }

    /// The earliest date the user has recorded a purchase.
    ///
    /// The burn rate measures the pace over the history the user actually
    /// has, which starts at their first purchase -- not at the start of the
    /// window, which would count days before they were recording as zero-spend
    /// days and understate the pace.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.first_purchase_date"))]
    async fn first_purchase_date(
        &self,
        user_id: Uuid,
        wallet_id: Option<Uuid>,
    ) -> Result<Option<chrono::NaiveDate>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;

        let row = conn
            .query_opt(
                "SELECT MIN(occurred_at)::date AS first_day
                   FROM purchase_fact
                  WHERE user_id = $1
                    AND ($2::uuid IS NULL OR wallet_id = $2)",
                &[&user_id, &wallet_id],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(row.and_then(|r| r.get("first_day")))
    }

    /// Compares projection totals to OLTP totals per (user, currency, UTC day).
    /// Any row returned is a projection bug, not a rounding tolerance.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.reconcile"))]
    async fn reconcile(&self) -> Result<Vec<ReconcileMismatch>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let rows = conn
            .query(
                r#"WITH fact AS (
                       SELECT user_id, base_currency, occurred_at::date AS day,
                              SUM(amount_base_minor) AS total
                         FROM purchase_fact
                        GROUP BY 1, 2, 3
                   ), oltp AS (
                       SELECT user_id, wallet_currency AS base_currency,
                              purchase_date::date AS day, SUM(total_wallet) AS total
                         FROM purchase_items
                        GROUP BY 1, 2, 3
                   )
                   SELECT COALESCE(f.user_id, o.user_id)             AS user_id,
                          COALESCE(f.base_currency, o.base_currency) AS base_currency,
                          COALESCE(f.day, o.day)                     AS day,
                          COALESCE(f.total, 0)::bigint               AS fact_total,
                          COALESCE(o.total, 0)::bigint               AS oltp_total
                     FROM fact f
                     FULL OUTER JOIN oltp o
                       ON f.user_id = o.user_id
                      AND f.base_currency = o.base_currency
                      AND f.day = o.day
                    WHERE COALESCE(f.total, 0) <> COALESCE(o.total, 0)"#,
                &[],
            )
            .await
            .map_err(RepositoryError::DbError)?;

        Ok(rows
            .iter()
            .map(|r| ReconcileMismatch {
                user_id: r.get("user_id"),
                base_currency: r.get("base_currency"),
                day: r.get("day"),
                fact_total: r.get("fact_total"),
                oltp_total: r.get("oltp_total"),
            })
            .collect())
    }
}

impl JobQueueRepository for PgAnalyticsRepo {
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.claim_due_job"))]
    async fn claim_due_job(&self, kind: &str) -> Result<Option<QueuedJob>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_opt(
                r#"UPDATE analytics_job
                      SET status = 'running', updated_at = NOW()
                    WHERE id = (
                        SELECT id FROM analytics_job
                         WHERE kind = $1
                           AND status = 'pending'
                           AND attempts < $2
                           AND run_after <= NOW()
                         ORDER BY run_after, id
                         LIMIT 1
                         FOR UPDATE SKIP LOCKED
                    )
                    RETURNING id, user_id, payload"#,
                &[&kind, &MAX_ATTEMPTS],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(row.map(|r| QueuedJob {
            id: r.get("id"),
            user_id: r.get("user_id"),
            payload: r.get("payload"),
        }))
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.complete_job"))]
    async fn complete_job(&self, id: i64) -> Result<()> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        conn.execute(
            "UPDATE analytics_job SET status = 'done', updated_at = NOW() WHERE id = $1",
            &[&id],
        )
        .await
        .map_err(RepositoryError::DbError)?;
        Ok(())
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.retry_job"))]
    async fn retry_job(&self, id: i64, error: &str) -> Result<u32> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_one(
                r#"UPDATE analytics_job
                      SET attempts = attempts + 1,
                          last_error = $2,
                          run_after = NOW() + make_interval(
                              mins => 5 * 2::int ^ LEAST(attempts, 6)
                          ),
                          status = CASE WHEN attempts + 1 >= $3 THEN 'failed' ELSE 'pending' END,
                          updated_at = NOW()
                    WHERE id = $1
                    RETURNING attempts"#,
                &[&id, &error, &MAX_ATTEMPTS],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(row.get::<_, i32>("attempts") as u32)
    }

    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.enqueue_resolve_jobs"))]
    async fn enqueue_resolve_jobs(&self) -> Result<u64> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        // One job per unresolved purchase that has no *open* job. Excluding only
        // 'pending' would resurrect exhausted ('failed') jobs on every sweep, which
        // would make the retry bound meaningless.
        let n = conn
            .execute(
                r#"INSERT INTO analytics_job (user_id, kind, payload)
                   SELECT f.user_id, 'resolve_rate',
                          jsonb_build_object('purchase_id', f.purchase_id::text)
                     FROM purchase_fact f
                    WHERE f.resolution = 'unresolved'
                      AND f.purchase_id IS NOT NULL
                      AND NOT EXISTS (
                          SELECT 1 FROM analytics_job j
                           WHERE j.kind = 'resolve_rate'
                             AND j.status IN ('pending', 'running')
                             AND j.payload->>'purchase_id' = f.purchase_id::text
                      )
                      AND NOT EXISTS (
                          SELECT 1 FROM analytics_job j
                           WHERE j.kind = 'resolve_rate'
                             AND j.status = 'failed'
                             AND j.payload->>'purchase_id' = f.purchase_id::text
                      )"#,
                &[],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(n)
    }
}
