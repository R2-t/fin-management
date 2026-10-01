use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    api::ApiError,
    application::inbound::repositories::PurchaseFilters,
    auth::AuthPrincipal,
    AppState,
};

use super::PurchaseResponse;

/// Default and maximum page sizes for list endpoints.
pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 200;

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub data: Vec<PurchaseResponse>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub merchant_id: Option<uuid::Uuid>,
    pub wallet_id: Option<uuid::Uuid>,
    pub category_id: Option<i32>,
    pub store: Option<String>,
    pub start_date: Option<chrono::DateTime<chrono::Utc>>,
    pub end_date: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    // Clamp instead of trusting the client: bounds the page and keeps a
    // negative offset from being turned into a SQL error.
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = params.offset.unwrap_or(0).max(0);

    let filters = PurchaseFilters {
        merchant_id: params.merchant_id,
        wallet_id: params.wallet_id,
        category_id: params.category_id,
        store: params.store,
        start_date: params.start_date,
        end_date: params.end_date,
        limit: Some(limit),
        offset: Some(offset),
    };

    let (purchases, total) = state
        .purchase_service
        .list(principal.user_id, filters)
        .await?;

    let body = ListResponse {
        data: purchases.into_iter().map(PurchaseResponse::from).collect(),
        total,
        limit,
        offset,
    };
    Ok((StatusCode::OK, Json(body)).into_response())
}