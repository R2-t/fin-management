//! Enqueuing analytics work for the Python worker.
//!
//! The application owns the write path (D4, D66): it inserts the job, the
//! sidecar claims and runs it. Every row carries the credential that names its
//! tenant (D72), minted here rather than by the worker -- the sidecar holds the
//! key to verify, and handing it the ability to mint would defeat the
//! separation.

use uuid::Uuid;

use crate::PgPool;

/// Why a job was not enqueued.
#[derive(Debug, thiserror::Error)]
pub enum EnqueueError {
    #[error("database error: {0}")]
    Database(String),
}

/// Enqueues a `classify` job for one purchase, and says whether it inserted one.
///
/// A second enqueue while a job is still open is a no-op, and the *database*
/// enforces that (D79): `uniq_analytics_job_open_purchase` is partial on the
/// statuses that compete, so a purchase can be classified again once the open
/// job finishes. The old `WHERE NOT EXISTS` guard was a scan, and two concurrent
/// statements both passed it.
///
/// The payload's `version` is the idempotency key the worker and the applier
/// both check (D65): a job that arrives after the purchase moved on is stale and
/// is dropped rather than applied.
pub async fn enqueue_classify(
    pool: &PgPool,
    user_id: Uuid,
    purchase_id: Uuid,
    version: i32,
    auth_token: &str,
) -> Result<bool, EnqueueError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| EnqueueError::Database(e.to_string()))?;

    let inserted = conn
        .execute(
            r#"INSERT INTO analytics_job (user_id, kind, payload, auth_token)
               VALUES ($1::uuid, 'classify',
                       jsonb_build_object('purchase_id', $2::text, 'version', $3::int),
                       $4::text)
               ON CONFLICT DO NOTHING"#,
            &[&user_id, &purchase_id.to_string(), &version, &auth_token],
        )
        .await
        .map_err(|e| EnqueueError::Database(e.to_string()))?;

    Ok(inserted > 0)
}

/// Mints a job credential and enqueues the suggestion for one purchase.
///
/// Shared by create and edit so the two paths cannot drift: without a service
/// key there is nothing to mint, so no row is written -- a job that can only
/// fail terminally would be worse than none.
pub async fn enqueue_suggestion(
    pool: &PgPool,
    auth: Option<&dyn crate::application::outbound::analytics::ServiceTokenSource>,
    user_id: Uuid,
    purchase_id: Uuid,
    version: i32,
) -> Result<bool, String> {
    let Some(source) = auth else {
        return Err("no service key is configured".into());
    };
    let token = source
        .token_for_job(user_id)
        .await
        .map_err(|e| format!("minting failed: {e}"))?;
    enqueue_classify(pool, user_id, purchase_id, version, &token)
        .await
        .map_err(|e| e.to_string())
}

/// Purchases that need a suggestion and have no job to produce one.
///
/// The create path enqueues, but a job's credential lives 300s (D72): a purchase
/// recorded while the sidecar was down has its job expire unclaimed, and nothing
/// else would ever try again. This is the repair (D78).
pub async fn purchases_needing_a_suggestion(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<(Uuid, Uuid, i32)>, super::RepositoryError> {
    let conn = pool.get().await.map_err(super::RepositoryError::PoolError)?;
    let rows = conn
        .query(
            r#"SELECT p.id, p.user_id, p.version
                 FROM purchase p
                WHERE p.category_id IS NULL
                  AND p.deleted_at IS NULL
                  AND NOT EXISTS (
                      SELECT 1 FROM analytics_job j
                       WHERE j.kind = 'classify'
                         AND j.status IN ('pending', 'running')
                         AND j.payload->>'purchase_id' = p.id::text
                  )
                  -- A purchase the sidecar has already answered about, and
                  -- answered "nothing honest to say", is not asked again on
                  -- every sweep. Only purchases with no prediction at all.
                  AND NOT EXISTS (
                      SELECT 1 FROM purchase_classification c
                       WHERE c.purchase_id = p.id
                  )
                ORDER BY p.created_at
                LIMIT $1"#,
            &[&limit],
        )
        .await
        .map_err(super::RepositoryError::DbError)?;

    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<_, Uuid>("id"),
                r.get::<_, Uuid>("user_id"),
                r.get::<_, i32>("version"),
            )
        })
        .collect())
}
