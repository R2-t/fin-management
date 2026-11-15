use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    /// `null` ungroups the wallet.
    pub group_id: Option<Uuid>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(wallet_id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .account_group_service
        .assign(principal.user_id, wallet_id, body.group_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
