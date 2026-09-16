use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, application::purchase::commands::RemoveItemCommand, AppState};

#[derive(Debug, Deserialize)]
pub struct PathParams {
    pub purchase_id: Uuid,
    pub sequence: i32,
}

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub user_id: Uuid,
    pub current_version: i32,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(params): Path<PathParams>,
    Query(query): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let cmd = RemoveItemCommand {
        id: params.purchase_id,
        user_id: query.user_id,
        current_version: query.current_version,
        sequence: params.sequence,
    };

    state.purchase_service.remove_item(cmd).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}
