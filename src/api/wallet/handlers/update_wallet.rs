use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::create_wallet::ResponseBody;
use crate::{api::ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub user_id: Uuid,
    pub name: String,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let wallet = state
        .wallet_service
        .update(id, body.user_id, body.name)
        .await?;

    Ok((StatusCode::OK, Json(ResponseBody::from(wallet))).into_response())
}
