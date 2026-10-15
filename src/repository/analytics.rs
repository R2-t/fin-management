use std::future::poll_fn;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use tokio::sync::Notify;
use tokio_postgres::{AsyncMessage, NoTls};
use tracing::instrument;
use uuid::Uuid;

use crate::application::inbound::repositories::Result;
use crate::repository::{PgPool, RepositoryError};

/// One claimed outbox row awaiting projection.
#[derive(Debug, Clone)]
pub struct PendingFact {
    pub outbox_id: i64,
    pub user_id: Uuid,
    pub purchase_id: Uuid,
}

/// A `(user, base_currency, day)` bucket where the projection disagrees with OLTP.
#[derive(Debug, Clone)]
pub struct ReconcileMismatch {
    pub user_id: Uuid,
    pub base_currency: String,
    pub day: NaiveDate,
    pub fact_total: i64,
    pub oltp_total: i64,
}

/// Rows are claimed with `SKIP LOCKED`; a claim older than this is reclaimable,
/// which also makes a crashed projector recover automatically.
const CLAIM_TIMEOUT_MINUTES: i32 = 5;
/// Give up (stop re-claiming) after this many attempts.
const MAX_ATTEMPTS: i32 = 10;

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

async fn listen_once(
    config: &tokio_postgres::Config,
    wake: &Arc<Notify>,
) -> std::result::Result<(), String> {
    let (client, mut connection) = config.connect(NoTls).await.map_err(|e| e.to_string())?;

    // The connection must be driven for LISTEN to complete and for
    // notifications to arrive; this task does both.
    let notify = wake.clone();
    let driver = tokio::spawn(async move {
        loop {
            match poll_fn(|cx| connection.poll_message(cx)).await {
                Some(Ok(AsyncMessage::Notification(_))) => notify.notify_one(),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.to_string()),
                None => return Ok(()),
            }
        }
    });

    tokio::time::timeout(
        Duration::from_secs(10),
        client.batch_execute(&format!("LISTEN {OUTBOX_CHANNEL}")),
    )
    .await
    .map_err(|_| "timed out issuing LISTEN".to_string())?
    .map_err(|e| e.to_string())?;

    tracing::info!(channel = OUTBOX_CHANNEL, "outbox listener ready");
    tracing::info!(gauge.outbox_listener_connected = 1);

    match driver.await {
        Ok(result) => result,
        Err(e) => Err(format!("listener driver task failed: {e}")),
    }
}

#[derive(Clone)]
pub struct PgAnalyticsRepo {
    pool: PgPool,
}

impl PgAnalyticsRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Atomically claims up to `limit` pending outbox rows.
    ///
    /// The `UPDATE ... WHERE id IN (SELECT ... FOR UPDATE SKIP LOCKED)` form
    /// takes the row locks and stamps `claimed_at` in one statement, so several
    /// projectors can run without processing the same row.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.claim_outbox"))]
    pub async fn claim_outbox(&self, limit: i64) -> Result<Vec<PendingFact>> {
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
    pub async fn mark_processed(&self, ids: &[i64]) -> Result<()> {
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
    pub async fn mark_failed(&self, id: i64, error: &str) -> Result<()> {
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
    pub async fn rebuild_fact(&self, user_id: Uuid, purchase_id: Uuid) -> Result<()> {
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
                      CASE WHEN v.conversion_resolved THEN 'completed' ELSE 'unresolved' END
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
    pub async fn enqueue_all_purchases(&self) -> Result<u64> {
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

    /// Age of the oldest unprocessed outbox row, in seconds.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.outbox_lag"))]
    pub async fn outbox_lag_seconds(&self) -> Result<Option<f64>> {
        let conn = self.pool.get().await.map_err(RepositoryError::PoolError)?;
        let row = conn
            .query_one(
                r#"SELECT EXTRACT(EPOCH FROM (NOW() - MIN(created_at)))::float8 AS lag
                     FROM analytics_outbox
                    WHERE processed_at IS NULL"#,
                &[],
            )
            .await
            .map_err(RepositoryError::DbError)?;
        Ok(row.get("lag"))
    }

    /// Compares projection totals to OLTP totals per (user, currency, UTC day).
    /// Any row returned is a projection bug, not a rounding tolerance.
    #[instrument(skip_all, err(level = "warn"), fields(db.system = "postgresql", db.operation = "analytics.reconcile"))]
    pub async fn reconcile(&self) -> Result<Vec<ReconcileMismatch>> {
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
