//! Outbound calls to the analytics sidecar.
//!
//! The sidecar owns the NLP and statistical work the Rust ecosystem does not
//! do well (D3). This module is the boundary: the application depends on the
//! trait, the HTTP client is one implementation, and a caller that cannot
//! reach the sidecar degrades to the deterministic answer rather than failing
//! the request.

use crate::application::error::ApplicationError;
use serde::{Deserialize, Serialize};

/// How long a call to the sidecar may take before it is abandoned.
///
/// The bands are a thousand resamples of thirty days, which is milliseconds
/// of work. A timeout much longer than that means the sidecar is gone, and
/// the caller is better served by the deterministic answer than by waiting.
pub const ANALYTICS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// The uncertainty bands around a runway projection.
///
/// Each percentile is one path through the resampled spending: P50 is the
/// most likely, P10 and P90 the edges of what is plausibly normal. An absent
/// `recovery_day` means that path never recovered within the horizon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayBands {
    pub percentile: i32,
    pub trough_minor: i64,
    pub recovery_day: Option<i64>,
    pub ending_balance_minor: i64,
}

/// The bands for one wallet, as the sidecar answered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayBandsResult {
    pub model_version: String,
    pub bands: Vec<RunwayBands>,
}

/// What the sidecar is asked, in the shape its endpoint takes.
#[derive(Debug, Clone, Serialize)]
pub struct RunwayBandsRequest {
    /// The tenant this answer is about. The sidecar compares it with the
    /// service token's claim and refuses a mismatch, so holding a token for one
    /// user cannot fetch another's bands.
    pub user_id: uuid::Uuid,
    /// Discretionary spend per day, oldest first, zero for days with none.
    pub daily_history: Vec<i64>,
    pub horizon: i64,
    pub recurring_by_day: std::collections::HashMap<i64, i64>,
    pub income_by_day: std::collections::HashMap<i64, i64>,
    pub starting_balance: i64,
}

/// A category suggestion, or an honest absence of one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyRequest {
    /// The tenant, compared against the claim; the claim decides it.
    pub user_id: uuid::Uuid,
    pub purchase_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyResult {
    /// `None` means "nothing honest to suggest", which is an answer, not a
    /// failure. A sidecar that could not look returns an error instead.
    pub category_id: Option<i32>,
    pub confidence: f64,
}

/// The analytics the sidecar provides.
///
/// The application depends on this trait, not on the HTTP client, so the
/// sidecar can be faked in tests and replaced in deployments.
/// Mints the credential a sidecar call carries (D68).
///
/// A trait so the HTTP client depends on the ability to mint, not on the key
/// material or the PASETO library.
#[async_trait::async_trait]
pub trait ServiceTokenSource: Send + Sync {
    /// A token for one HTTP call: short-lived, because the call is immediate.
    async fn token_for(&self, user_id: uuid::Uuid) -> Result<String, String>;

    /// A token for a queued job (D72): longer-lived, because a job is not
    /// claimed instantly. Minting a call token for a job makes the job fail
    /// terminally when it is claimed after the call token expired.
    async fn token_for_job(&self, user_id: uuid::Uuid) -> Result<String, String> {
        self.token_for(user_id).await
    }
}

/// So `Arc<dyn SpendAnalytics>` can stand in for the concrete implementation.
#[async_trait::async_trait]
impl SpendAnalytics for std::sync::Arc<dyn SpendAnalytics> {
    async fn runway_bands(
        &self,
        request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, crate::application::error::ApplicationError> {
        (**self).runway_bands(request).await
    }
}

#[async_trait::async_trait]
pub trait SpendAnalytics: Send + Sync {
    /// A category suggestion for one purchase, computed inline (D67).
    ///
    /// The caller bounds this by a deadline and falls back to the job queue, so
    /// recording a purchase never waits on it.
    async fn classify(
        &self,
        request: ClassifyRequest,
    ) -> Result<Option<ClassifyResult>, crate::application::error::ApplicationError> {
        let _ = request;
        Ok(None)
    }

    /// The uncertainty bands around a runway projection.
    ///
    /// Returns `None` when the sidecar could not be reached: the caller
    /// answers deterministically rather than failing, because a forecast
    /// without bands is better than no forecast.
    async fn runway_bands(
        &self,
        request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, ApplicationError>;
}
