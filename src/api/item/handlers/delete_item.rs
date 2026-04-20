use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError,
    application::inbound::repositories::ItemRepository,
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct PathParams {
    pub purchase_id: Uuid,
    pub sequence: i32,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(params): Path<PathParams>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .item_repo
        .delete(params.purchase_id, params.sequence)
        .await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}
