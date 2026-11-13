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
    /// Discretionary spend per day, oldest first, zero for days with none.
    pub daily_history: Vec<i64>,
    pub horizon: i64,
    pub recurring_by_day: std::collections::HashMap<i64, i64>,
    pub income_by_day: std::collections::HashMap<i64, i64>,
    pub starting_balance: i64,
}

/// The analytics the sidecar provides.
///
/// The application depends on this trait, not on the HTTP client, so the
/// sidecar can be faked in tests and replaced in deployments.
#[async_trait::async_trait]
pub trait SpendAnalytics: Send + Sync {
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
