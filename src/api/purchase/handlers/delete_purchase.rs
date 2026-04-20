use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub user_id: Uuid,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    state.purchase_service.delete(id, params.user_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
