use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::application::inbound::repositories::{SpendRange, TrendBucket};
use crate::application::trends::service::TrendsQuery;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub bucket: String,
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    /// Optional: report a single currency.
    pub currency: Option<String>,
}

impl QueryParams {
    fn bucket(&self) -> Result<TrendBucket, ApiError> {
        match self.bucket.to_ascii_lowercase().as_str() {
            "day" => Ok(TrendBucket::Day),
            "week" => Ok(TrendBucket::Week),
            "month" => Ok(TrendBucket::Month),
            other => Err(ApiError::BadRequest(format!(
                "unknown bucket {other:?}: expected day, week or month"
            ))),
        }
    }
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    axum::extract::Query(params): axum::extract::Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let bucket = params.bucket()?;
    super::super::validate_range(params.from, params.to)?;

    // Buckets belong to the user's calendar, not UTC's (D25).
    let timezone = state
        .user_profile_service
        .get(principal.user_id)
        .await?
        .timezone;

    let query = TrendsQuery {
        range: SpendRange {
            from: params.from,
            to: params.to,
            wallet_id: params.wallet_id,
            category_id: params.category_id,
            currency: params.currency,
            timezone,
        },
        bucket,
    };

    let view = state
        .trends_service
        .trends(principal.user_id, query)
        .await?;
    Ok((StatusCode::OK, Json(view)).into_response())
}
