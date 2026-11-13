//! The HTTP implementation of [`SpendAnalytics`].
//!
//! One timeout, no retry: a forecast endpoint is read-only and idempotent on
//! the sidecar's side, but the caller already has a deterministic answer, so
//! the right response to an unreachable sidecar is to use it rather than wait
//! for a second failure.

use std::time::Duration;

use crate::application::error::ApplicationError;
use crate::application::outbound::analytics::{
    RunwayBandsRequest, RunwayBandsResult, SpendAnalytics, ANALYTICS_TIMEOUT,
};

/// Calls the sidecar over HTTP.
pub struct HttpAnalyticsClient {
    base_url: String,
    client: reqwest::Client,
}

impl HttpAnalyticsClient {
    /// Builds a client for a sidecar at `base_url`, e.g. `http://analytics:8000`.
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(ANALYTICS_TIMEOUT)
            .build()
            .expect("a valid reqwest client");
        Self { base_url, client }
    }
}

#[async_trait::async_trait]
impl SpendAnalytics for HttpAnalyticsClient {
    async fn runway_bands(
        &self,
        request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, ApplicationError> {
        let response = self
            .client
            .post(format!("{}/runway-bands", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(|e| ApplicationError::Internal(format!("analytics sidecar unreachable: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApplicationError::Internal(format!(
                "analytics sidecar returned {status}: {body}"
            )));
        }

        let parsed: RunwayBandsResult = response
            .json()
            .await
            .map_err(|e| ApplicationError::Internal(format!("analytics sidecar returned unreadable bands: {e}")))?;

        Ok(Some(parsed))
    }
}

/// A client that answers as if the sidecar had declined, for tests and for
/// deployments that run without one.
///
/// The deterministic forecast is the fallback, so a missing sidecar must look
/// like "no bands" rather than like a failure -- the caller's fallback logic
/// lives above this trait, not inside it.
pub struct NoAnalytics;

#[async_trait::async_trait]
impl SpendAnalytics for NoAnalytics {
    async fn runway_bands(
        &self,
        _request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, ApplicationError> {
        Ok(None)
    }
}

/// Kept as the module's stated timeout contract, referenced by tests.
pub fn analytics_timeout() -> Duration {
    ANALYTICS_TIMEOUT
}
