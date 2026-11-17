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
    pub reason: Option<String>,
}

/// Dismissing is permanent until the user undismisses: the next sweep must not
/// ask the same question again.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
    body: Option<Json<RequestBody>>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .anomaly_service
        .dismiss(
            principal.user_id,
            id,
            body.and_then(|Json(body)| body.reason),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
