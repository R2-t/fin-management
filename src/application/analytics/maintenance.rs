use std::time::Duration;

use crate::repository::analytics::PgAnalyticsRepo;

/// Purges processed outbox rows older than `keep_for`.
///
/// Returns how many rows were removed. Pending rows are never removed.
pub async fn purge_processed_outbox(
    repo: &PgAnalyticsRepo,
    keep_for: Duration,
) -> Result<u64, String> {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(keep_for).map_err(|e| e.to_string())?;
    repo.purge_processed_before(cutoff)
        .await
        .map_err(|e| e.to_string())
}

/// Rows that have exhausted their retries and are no longer claimed.
pub async fn stuck_outbox_report(
    repo: &PgAnalyticsRepo,
) -> Result<Vec<crate::repository::analytics::StuckOutboxItem>, String> {
    repo.stuck_outbox().await.map_err(|e| e.to_string())
}
