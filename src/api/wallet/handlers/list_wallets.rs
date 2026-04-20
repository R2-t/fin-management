use axum::{extract::{Query, State}, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, AppState};
use super::create_wallet::ResponseBody;

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub user_id: Uuid,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let wallets = state.wallet_service.list(params.user_id).await?;
    let body: Vec<ResponseBody> = wallets.into_iter().map(ResponseBody::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}
