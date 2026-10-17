use tracing::warn;

use chrono::{DateTime, Timelike, Utc};
use tracing::{error, info, instrument};

use crate::application::error::ApplicationError;
use crate::application::inbound::repositories::{
    CompletionError, JobQueueRepository, PurchaseRepository, RateCacheRepository,
    RateHistoryReader, RateHistoryWriter, WalletRepository,
};
use crate::application::outbound::exchange_rate::ExchangeRateProvider;

/// Completes Purchases that were recorded without a conversion (ADR-0006).
///
/// Drives `analytics_job(kind = 'resolve_rate')`: claims due jobs, resolves the
/// rate cache-first then via the provider, updates the Purchase through the
/// normal versioned path, and relies on the outbox to rebuild the projection.
/// Failures back off exponentially and are bounded; an exhausted job is marked
/// `failed` and surfaced by the stuck-outbox-style reporting.
///
/// Generic over the storage and provider contracts, so it can be driven by fakes.
pub struct RateCompletionWorker<J, P, W, R, F>
where
    J: JobQueueRepository + Sync,
    P: PurchaseRepository + Sync,
    W: WalletRepository + Sync,
    R: RateCacheRepository + RateHistoryReader + RateHistoryWriter + Sync,
    F: ExchangeRateProvider + Sync,
{
    jobs: J,
    purchases: P,
    wallets: W,
    rate_cache: R,
    fx: F,
}

impl<J, P, W, R, F> RateCompletionWorker<J, P, W, R, F>
where
    J: JobQueueRepository + Sync,
    P: PurchaseRepository + Sync,
    W: WalletRepository + Sync,
    R: RateCacheRepository + RateHistoryReader + RateHistoryWriter + Sync,
    F: ExchangeRateProvider + Sync,
{
    pub fn new(jobs: J, purchases: P, wallets: W, rate_cache: R, fx: F) -> Self {
        Self {
            jobs,
            purchases,
            wallets,
            rate_cache,
            fx,
        }
    }

    /// Claims and handles one due job. Returns `true` when a job was processed.
    ///
    /// Not instrumented per call: `run_forever` provides the lifetime span.
    pub async fn run_once(&self) -> Result<bool, ApplicationError> {
        let Some(job) = self.jobs.claim_due_job("resolve_rate").await? else {
            return Ok(false);
        };

        let purchase_id: Option<uuid::Uuid> = job
            .payload
            .get("purchase_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        let Some(purchase_id) = purchase_id else {
            // Malformed payload: not retryable.
            error!(job_id = job.id, "resolve_rate job has no purchase_id");
            // A malformed payload cannot be fixed by retrying; `retry_job` still
            // exhausts it after MAX_ATTEMPTS, which surfaces it rather than
            // retrying forever.
            let _ = self.jobs.retry_job(job.id, "malformed payload").await;
            return Ok(true);
        };

        match self.complete(&job.user_id, purchase_id).await {
            Ok(()) => {
                self.jobs.complete_job(job.id).await?;
                info!(
                    job_id = job.id,
                    purchase_id = %purchase_id,
                    "unresolved rate completed"
                );
                tracing::info!(monotonic_counter.unresolved_rate_completed_total = 1u64);
            }
            Err(e) => {
                let attempts = self.jobs.retry_job(job.id, &e.to_string()).await?;
                warn!(
                    job_id = job.id,
                    attempts,
                    error = %e,
                    "rate completion failed; scheduled for retry"
                );
                tracing::info!(monotonic_counter.rate_completion_failure_total = 1u64);
            }
        }

        Ok(true)
    }

    /// Cache first (any age), then the durable history, then the provider at the
    /// purchase's own date. The provider's answer is recorded to both so the next
    /// resolution succeeds even after a restart empties the unlogged cache.
    async fn resolve(
        &self,
        from: &str,
        to: &str,
        at: DateTime<Utc>,
    ) -> Result<crate::application::inbound::repositories::RateUpdate, CompletionError> {
        if let Some(cached) = self.rate_cache.get_most_recent(from, to).await? {
            return Ok(crate::application::inbound::repositories::RateUpdate {
                rate_scaled: cached.rate_scaled,
                source: crate::application::currency::RateSource::Resolved
                    .as_str()
                    .to_string(),
                at: cached.rate_hour,
            });
        }

        if let Some(recorded) = self.rate_cache.latest_history(from, to).await? {
            return Ok(crate::application::inbound::repositories::RateUpdate {
                rate_scaled: recorded.rate_scaled,
                source: crate::application::currency::RateSource::Resolved
                    .as_str()
                    .to_string(),
                at: recorded.rate_hour,
            });
        }

        let result = self
            .fx
            .fetch_rate(crate::application::outbound::exchange_rate::RateRequest {
                from: from.to_string(),
                to: to.to_string(),
                at,
            })
            .await
            .map_err(CompletionError::Rate)?;
        let rate_scaled = crate::application::currency::rate_to_scaled(result.rate)?;

        // Record what we just learned, so a later retry (or a restart) does not
        // have to call the provider again.
        let hour = at
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(at);
        self.rate_cache
            .record_history(from, to, hour, rate_scaled, &result.source)
            .await?;
        self.rate_cache
            .put(from, to, hour, rate_scaled, &result.source)
            .await?;

        Ok(crate::application::inbound::repositories::RateUpdate {
            rate_scaled,
            source: crate::application::currency::RateSource::Resolved.as_str().to_string(),
            at: result.fetched_at,
        })
    }

    async fn complete(
        &self,
        user_id: &uuid::Uuid,
        purchase_id: uuid::Uuid,
    ) -> Result<(), CompletionError> {
        let purchase = self
            .purchases
            .get(purchase_id, *user_id)
            .await
            .map_err(CompletionError::Application)?;

        // Already completed meanwhile (e.g. the user typed the rate): no-op.
        if purchase.exchange_rate_scaled.is_some() {
            return Ok(());
        }

        let wallet = self
            .wallets
            .get(*user_id, purchase.wallet_id)
            .await
            .map_err(CompletionError::Application)?;

        if purchase.currency == wallet.currency {
            // The purchase no longer needs a conversion (or never did); complete it
            // as not-required by leaving the rate absent.
            return Ok(());
        }

        let rate = self.resolve(&purchase.currency, &wallet.currency, purchase.purchase_date).await?;

        self.purchases
            .update(
                purchase_id,
                *user_id,
                purchase.version,
                crate::application::inbound::repositories::PurchaseUpdates {
                    merchant_id: None,
                    store: None,
                    description: None,
                    notes: None,
                    category_id: None,
                    purchase_date: None,
                    rate: Some(rate),
                },
                None,
            )
            .await
            .map_err(CompletionError::Application)?;

        Ok(())
    }
}
impl<J, P, W, R, F> RateCompletionWorker<J, P, W, R, F>
where
    J: JobQueueRepository + Sync,
    P: PurchaseRepository + Sync,
    W: WalletRepository + Sync,
    R: RateCacheRepository + RateHistoryReader + RateHistoryWriter + Sync,
    F: ExchangeRateProvider + Sync,
{
    /// Safety net: queue a completion job for any unresolved Purchase that has
    /// no open job. Cheap when there is nothing to do.
    #[instrument(skip(self))]
    pub async fn sweep(&self) -> Result<u64, ApplicationError> {
        self.jobs.enqueue_resolve_jobs().await
    }

    /// Drain due jobs, sweeping for unresolved purchases about once an hour.
    ///
    /// Instrumented at lifetime level: `run_once` is called in a tight loop, so
    /// per-iteration spans would be continuous noise (OBSERVABILITY.md).
    #[instrument(skip_all)]
    pub async fn run_forever(self, idle: std::time::Duration, sweep_every: std::time::Duration) {
        let mut since_sweep = std::time::Instant::now();
        loop {
            match self.run_once().await {
                Ok(true) => {}
                Ok(false) => {
                    if since_sweep.elapsed() >= sweep_every {
                        match self.sweep().await {
                            Ok(n) if n > 0 => info!(queued = n, "queued rate-completion jobs"),
                            Ok(_) => {}
                            Err(e) => error!(error = %e, "rate-completion sweep failed"),
                        }
                        since_sweep = std::time::Instant::now();
                    }
                    tokio::time::sleep(idle).await;
                }
                Err(e) => {
                    error!(error = %e, "rate completion worker failed");
                    tokio::time::sleep(idle).await;
                }
            }
        }
    }
}
