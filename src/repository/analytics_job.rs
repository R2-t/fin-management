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
/// A second enqueue for a purchase that already has an open job is a no-op, so a
/// retried create does not double the work. The payload's `version` is the
/// idempotency key the worker checks (D65): a job that arrives after the
/// purchase moved on is stale and is skipped.
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
               SELECT $1::uuid, 'classify',
                      jsonb_build_object('purchase_id', $2::text, 'version', $3::int),
                      $4::text
                WHERE NOT EXISTS (
                    SELECT 1 FROM analytics_job
                     WHERE kind = 'classify'
                       AND status IN ('pending', 'running')
                       AND payload->>'purchase_id' = $2::text
                )"#,
            &[&user_id, &purchase_id.to_string(), &version, &auth_token],
        )
        .await
        .map_err(|e| EnqueueError::Database(e.to_string()))?;

    Ok(inserted > 0)
}
