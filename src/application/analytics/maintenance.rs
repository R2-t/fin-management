use std::time::Duration;

use tracing::instrument;

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{AnalyticsRepository, StuckOutboxItem};

/// Purges processed outbox rows older than `keep_for`.
///
/// Returns how many rows were removed. Pending rows are never removed.
#[instrument(skip_all, err(level = "warn"))]
pub async fn purge_processed_outbox<R: AnalyticsRepository>(
    repo: &R,
    keep_for: Duration,
) -> Result<u64, ApplicationError> {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(keep_for).map_err(|e| ApplicationError::Validation(format!(
            "invalid retention window: {e}"
        )))?;
    repo.purge_processed_before(cutoff).await
}

/// Rows that have exhausted their retries and are no longer claimed.
#[instrument(skip_all, err(level = "warn"))]
pub async fn stuck_outbox_report<R: AnalyticsRepository>(
    repo: &R,
) -> Result<Vec<StuckOutboxItem>, ApplicationError> {
    repo.stuck_outbox().await
}
