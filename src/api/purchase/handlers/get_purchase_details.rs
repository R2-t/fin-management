use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, AppState};

use super::PurchaseResponse;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub user_id: Uuid,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let purchase = state.purchase_service.get(id, params.user_id).await?;
    Ok((StatusCode::OK, Json(PurchaseResponse::from(purchase))).into_response())
}
