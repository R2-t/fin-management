use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub name: String,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let group = state
        .account_group_service
        .rename(principal.user_id, id, body.name)
        .await?;
    Ok(Json(group))
}
