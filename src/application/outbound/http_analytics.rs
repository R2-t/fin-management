//! The HTTP implementation of [`SpendAnalytics`].
//!
//! One timeout, no retry: a forecast endpoint is read-only and idempotent on
//! the sidecar's side, but the caller already has a deterministic answer, so
//! the right response to an unreachable sidecar is to use it rather than wait
//! for a second failure.

use std::time::Duration;

use crate::application::error::ApplicationError;
use std::sync::Arc;

use crate::application::outbound::analytics::{
    ClassifyRequest, ClassifyResult, RunwayBandsRequest, RunwayBandsResult, ServiceTokenSource,
    SpendAnalytics, ANALYTICS_TIMEOUT,
};

/// Calls the sidecar over HTTP.
pub struct HttpAnalyticsClient {
    base_url: String,
    client: reqwest::Client,
    /// Mints the credential each call carries (D68).
    ///
    /// Optional so a deployment without a configured key still serves: the call
    /// goes out unauthenticated and the sidecar refuses it, which is visible in
    /// the logs as `service_token_rejected` rather than as a silent fallback.
    auth: Option<Arc<dyn ServiceTokenSource>>,
}

impl HttpAnalyticsClient {
    /// Builds a client for a sidecar at `base_url`, e.g. `http://analytics:8000`.
    pub fn new(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(ANALYTICS_TIMEOUT)
            .build()
            .expect("a valid reqwest client");
        Self {
            base_url,
            client,
            auth: None,
        }
    }

    /// The same client, with a credential minted per call for the tenant being
    /// served.
    pub fn with_auth(mut self, auth: Arc<dyn ServiceTokenSource>) -> Self {
        self.auth = Some(auth);
        self
    }

    async fn authorize(
        &self,
        request: reqwest::RequestBuilder,
        user_id: uuid::Uuid,
    ) -> Result<reqwest::RequestBuilder, ApplicationError> {
        match &self.auth {
            None => Ok(request),
            Some(source) => {
                let token = source
                    .token_for(user_id)
                    .await
                    .map_err(|e| ApplicationError::Internal(format!("service token: {e}")))?;
                Ok(request.bearer_auth(token))
            }
        }
    }
}

#[async_trait::async_trait]
impl SpendAnalytics for HttpAnalyticsClient {
    async fn classify(
        &self,
        request: ClassifyRequest,
    ) -> Result<Option<ClassifyResult>, ApplicationError> {
        let user_id = request.user_id;
        let builder = self
            .client
            .post(format!("{}/classify", self.base_url))
            .json(&request);
        let response = self
            .authorize(builder, user_id)
            .await?
            .send()
            .await
            .map_err(|e| {
                ApplicationError::Internal(format!("analytics sidecar unreachable: {e}"))
            })?;

        // A refused token or a failed sidecar is an OUTAGE, not an empty
        // answer. Returning Ok(None) here would read as "the sidecar looked and
        // had nothing to say", and the caller would skip the queue -- leaving
        // the purchase permanently unclassified.
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            tracing::warn!(
                reason = "service_token_rejected",
                "the analytics sidecar refused the service token"
            );
            return Err(ApplicationError::Internal(
                "the analytics sidecar refused the service token".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(ApplicationError::Internal(format!(
                "the analytics sidecar answered {}",
                response.status()
            )));
        }

        let body: ClassifyResult = response.json().await.map_err(|e| {
            ApplicationError::Internal(format!("analytics sidecar answered badly: {e}"))
        })?;
        Ok(Some(body))
    }

    async fn runway_bands(
        &self,
        request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, ApplicationError> {
        let request_builder = self
            .client
            .post(format!("{}/runway-bands", self.base_url))
            .json(&request);
        let response = self
            .authorize(request_builder, request.user_id)
            .await?
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
    async fn classify(
        &self,
        _request: ClassifyRequest,
    ) -> Result<Option<ClassifyResult>, ApplicationError> {
        // "There is no sidecar" is an outage, not an empty answer: the caller
        // must fall back to the queue rather than conclude there is nothing to
        // suggest.
        Err(ApplicationError::Internal(
            "no analytics backend is configured".into(),
        ))
    }

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
