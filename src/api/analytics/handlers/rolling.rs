use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::application::inbound::repositories::SpendRange;
use crate::application::trends::service::RollingQuery;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub window_days: i32,
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    axum::extract::Query(params): axum::extract::Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    if params.from >= params.to {
        return Err(ApiError::BadRequest(
            "`from` must be earlier than `to`".into(),
        ));
    }
    if !(1..=365).contains(&params.window_days) {
        return Err(ApiError::BadRequest(
            "`window_days` must be between 1 and 365".into(),
        ));
    }

    let timezone = state
        .user_profile_service
        .get(principal.user_id)
        .await?
        .timezone;

    let query = RollingQuery {
        range: SpendRange {
            from: params.from,
            to: params.to,
            wallet_id: params.wallet_id,
            category_id: params.category_id,
            timezone,
        },
        window_days: params.window_days,
    };

    let view = state
        .trends_service
        .rolling(principal.user_id, query)
        .await?;
    Ok((StatusCode::OK, Json(view)).into_response())
}
