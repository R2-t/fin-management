use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::{
    api::ApiError,
    application::inbound::repositories::PurchaseFilters,
    auth::AuthPrincipal,
    AppState,
};

use super::PurchaseResponse;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
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
    let filters = PurchaseFilters {
        wallet_id: params.wallet_id,
        category_id: params.category_id,
        store: params.store,
        start_date: params.start_date,
        end_date: params.end_date,
        limit: params.limit,
        offset: params.offset,
    };

    let purchases = state
        .purchase_service
        .list(principal.user_id, filters)
        .await?;

    let body: Vec<PurchaseResponse> = purchases.into_iter().map(PurchaseResponse::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}