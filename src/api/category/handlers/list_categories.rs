use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::create_category::ResponseBody;
use crate::{api::ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub user_id: Option<Uuid>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let categories = state.category_service.list(params.user_id).await?;
    let body: Vec<ResponseBody> = categories.into_iter().map(ResponseBody::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}
