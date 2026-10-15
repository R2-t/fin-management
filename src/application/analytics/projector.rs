use std::collections::HashSet;
use std::time::Duration;

use tracing::{error, info};

use crate::repository::analytics::PgAnalyticsRepo;

/// Drains `analytics_outbox` into `purchase_fact`.
///
/// Processing is idempotent: each outbox row only names a purchase, and the
/// projector re-reads the current OLTP state, so replays and duplicates are
/// harmless and several rows for one purchase collapse into one rebuild.
pub struct FactProjector {
    repo: PgAnalyticsRepo,
    batch_size: i64,
}

impl FactProjector {
    pub fn new(repo: PgAnalyticsRepo) -> Self {
        Self {
            repo,
            batch_size: 500,
        }
    }

    /// Processes one batch. Returns the number of outbox rows completed.
    pub async fn run_once(&self) -> Result<usize, String> {
        let pending = self
            .repo
            .claim_outbox(self.batch_size)
            .await
            .map_err(|e| e.to_string())?;

        if pending.is_empty() {
            return Ok(0);
        }

        let mut done: Vec<i64> = Vec::new();
        // Many outbox rows can point at the same purchase (several edits); one
        // rebuild covers all of them.
        let mut rebuilt: HashSet<uuid::Uuid> = HashSet::new();

        for item in &pending {
            if !rebuilt.insert(item.purchase_id) {
                done.push(item.outbox_id);
                continue;
            }
            match self.repo.rebuild_fact(item.user_id, item.purchase_id).await {
                Ok(()) => done.push(item.outbox_id),
                Err(e) => {
                    error!(error = %e, purchase_id = %item.purchase_id, "fact projection failed");
                    let _ = self.repo.mark_failed(item.outbox_id, &e.to_string()).await;
                }
            }
        }

        let count = done.len();
        self.repo
            .mark_processed(&done)
            .await
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    /// Backfill helper: drains until the outbox is empty.
    pub async fn drain(&self) -> Result<usize, String> {
        let mut total = 0;
        loop {
            let n = self.run_once().await?;
            if n == 0 {
                return Ok(total);
            }
            total += n;
        }
    }

    /// Background loop. Emits projection counts and outbox lag.
    ///
    /// When the outbox is empty it waits for a notification, with the poll
    /// interval as a mandatory fallback: notifications are not queued for
    /// absent listeners, so a missed signal must still be caught.
    pub async fn run_forever(self, idle: Duration, wake: std::sync::Arc<tokio::sync::Notify>) {
        loop {
            match self.run_once().await {
                Ok(0) => {
                    tokio::select! {
                        _ = wake.notified() => {}
                        _ = tokio::time::sleep(idle) => {}
                    }
                }
                Ok(n) => {
                    info!(monotonic_counter.fact_projected_total = n as u64);
                }
                Err(e) => {
                    error!(error = %e, "projector batch failed");
                    tokio::time::sleep(idle).await;
                }
            }

            if let Ok(Some(lag)) = self.repo.outbox_lag_seconds().await {
                info!(gauge.analytics_outbox_lag_seconds = lag);
            }

            // Exhausted rows are no longer pending work and no longer counted in
            // lag, so they must be surfaced explicitly or they vanish silently.
            if let Ok(stuck) = self.repo.stuck_outbox().await {
                if stuck.is_empty() {
                    info!(gauge.analytics_outbox_dead_letter = 0u64);
                } else {
                    // The count goes in the log, not a metric label (bounded labels only).
                    info!(gauge.analytics_outbox_dead_letter = stuck.len() as u64);
                    error!(
                        count = stuck.len(),
                        "outbox rows have exhausted their retries and are no longer claimed"
                    );
                }
            }
        }
    }
}
