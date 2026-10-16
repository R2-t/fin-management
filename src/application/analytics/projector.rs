use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tracing::{error, info};

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::AnalyticsRepository;

/// Drains `analytics_outbox` into `purchase_fact`.
///
/// Processing is idempotent: each outbox row only names a purchase, and the
/// projector re-reads the current OLTP state, so replays and duplicates are
/// harmless and several rows for one purchase collapse into one rebuild.
/// Generic over the storage contract, so the projector can be driven by a fake
/// in tests and never depends on a concrete implementation.
pub struct FactProjector<R: AnalyticsRepository> {
    repo: R,
    batch_size: i64,
}

/// Set when the projector has purged since process start, so a long-lived loop
/// purges once per day rather than once per hour.
static PURGED_SINCE_START: AtomicBool = AtomicBool::new(false);

impl<R: AnalyticsRepository + Send + Sync + 'static> FactProjector<R> {
    pub fn new(repo: R) -> Self {
        Self {
            repo,
            batch_size: 500,
        }
    }

    /// Processes one batch. Returns the number of outbox rows completed.
    ///
    /// Not instrumented per call: this runs once a second when idle, so a span
    /// here would be continuous noise. The lifetime-level spans cover it; the
    /// exception is recorded in docs/OBSERVABILITY.md.
    pub async fn run_once(&self) -> Result<usize, ApplicationError> {
        let pending = self.repo.claim_outbox(self.batch_size).await?;

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
        self.repo.mark_processed(&done).await?;
        Ok(count)
    }

    /// Backfill helper: drains until the outbox is empty.
    #[tracing::instrument(skip_all, err(level = "warn"))]
    pub async fn drain(&self) -> Result<usize, ApplicationError> {
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
    #[tracing::instrument(skip_all)]
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

            // First tick performs the retention pass once; the manual command
            // remains available for operator-driven purges.
            if !PURGED_SINCE_START.swap(true, Ordering::Relaxed) {
                let keep_days = std::env::var("PURGE_KEEP_DAYS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(7);
                match crate::application::analytics::maintenance::purge_processed_outbox(
                    &self.repo,
                    Duration::from_secs(keep_days * 24 * 3600),
                )
                .await
                {
                    Ok(n) if n > 0 => info!(removed = n, keep_days, "outbox purged"),
                    Ok(_) => {}
                    Err(e) => error!(error = %e, "scheduled outbox purge failed"),
                }
            }

            // Exhausted rows are no longer pending work and no longer counted in
            // lag, so they must be surfaced explicitly or they vanish silently.
            match self.repo.stuck_outbox().await {
                Ok(stuck) => {
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
                Err(e) => error!(error = %e, "could not read stuck outbox rows"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::inbound::repositories::{PendingFact, StuckOutboxItem};
    use std::sync::Mutex;
    use uuid::Uuid;

    /// In-memory stand-in for the analytics storage: the projector now depends on
    /// the interface, so its behaviour can be exercised without a database.
    #[derive(Default)]
    struct FakeRepo {
        batches: Mutex<Vec<Vec<PendingFact>>>,
        rebuilt: Mutex<Vec<Uuid>>,
        processed: Mutex<Vec<i64>>,
        failed: Mutex<Vec<(i64, String)>>,
        fail_rebuild: bool,
    }

    impl AnalyticsRepository for FakeRepo {
        async fn claim_outbox(&self, _limit: i64) -> Result<Vec<PendingFact>, ApplicationError> {
            let mut batches = self.batches.lock().unwrap();
            if batches.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(batches.remove(0))
            }
        }
        async fn rebuild_fact(&self, _user: Uuid, purchase_id: Uuid) -> Result<(), ApplicationError> {
            if self.fail_rebuild {
                return Err(ApplicationError::Internal("projection exploded".into()));
            }
            self.rebuilt.lock().unwrap().push(purchase_id);
            Ok(())
        }
        async fn mark_processed(&self, ids: &[i64]) -> Result<(), ApplicationError> {
            self.processed.lock().unwrap().extend_from_slice(ids);
            Ok(())
        }
        async fn mark_failed(&self, id: i64, error: &str) -> Result<(), ApplicationError> {
            self.failed.lock().unwrap().push((id, error.to_string()));
            Ok(())
        }
        async fn enqueue_all_purchases(&self) -> Result<u64, ApplicationError> {
            Ok(0)
        }
        async fn purge_processed_before(
            &self,
            _cutoff: chrono::DateTime<chrono::Utc>,
        ) -> Result<u64, ApplicationError> {
            Ok(0)
        }
        async fn stuck_outbox(&self) -> Result<Vec<StuckOutboxItem>, ApplicationError> {
            Ok(Vec::new())
        }
        async fn unresolved_purchases(
            &self,
        ) -> Result<Vec<crate::application::inbound::repositories::UnresolvedPurchase>, ApplicationError>
        {
            Ok(Vec::new())
        }
        async fn outbox_lag_seconds(&self) -> Result<Option<f64>, ApplicationError> {
            Ok(None)
        }
        async fn reconcile(&self) -> Result<Vec<crate::application::inbound::repositories::ReconcileMismatch>, ApplicationError> {
            Ok(Vec::new())
        }
    }

    fn fact(outbox_id: i64, purchase_id: Uuid) -> PendingFact {
        PendingFact {
            outbox_id,
            user_id: Uuid::new_v4(),
            purchase_id,
        }
    }

    /// Several edits to one Purchase collapse into a single rebuild, and every
    /// event is still marked processed.
    #[tokio::test]
    async fn multiple_events_for_one_purchase_collapse_into_one_rebuild() {
        let purchase = Uuid::new_v4();
        let repo = FakeRepo {
            batches: Mutex::new(vec![vec![fact(1, purchase), fact(2, purchase), fact(3, purchase)]]),
            ..Default::default()
        };
        let projector = FactProjector::new(repo);

        let handled = projector.run_once().await.expect("batch");

        assert_eq!(handled, 3, "all three events are completed");
        assert_eq!(
            projector.repo.rebuilt.lock().unwrap().len(),
            1,
            "one rebuild covers every event for the same Purchase"
        );
        let mut processed = projector.repo.processed.lock().unwrap().clone();
        processed.sort();
        assert_eq!(processed, vec![1, 2, 3]);
    }

    /// A rebuild that fails is recorded against its event and is NOT marked
    /// processed, so the work is retried rather than silently dropped.
    #[tokio::test]
    async fn a_failing_rebuild_is_recorded_and_not_marked_processed() {
        let repo = FakeRepo {
            batches: Mutex::new(vec![vec![fact(7, Uuid::new_v4())]]),
            fail_rebuild: true,
            ..Default::default()
        };
        let projector = FactProjector::new(repo);

        let handled = projector.run_once().await.expect("batch");

        assert_eq!(handled, 0, "a failed rebuild is not completed work");
        assert!(projector.repo.processed.lock().unwrap().is_empty());
        let failed = projector.repo.failed.lock().unwrap().clone();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, 7);
        assert!(failed[0].1.contains("projection exploded"));
    }
}
