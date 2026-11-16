use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;

use super::create_category::ResponseBody;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> Result<impl IntoResponse, ApiError> {
    // Some(principal) => system categories plus the caller's own.
    let categories = state.category_service.list(Some(principal.user_id)).await?;
    let body: Vec<ResponseBody> = categories.into_iter().map(ResponseBody::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}