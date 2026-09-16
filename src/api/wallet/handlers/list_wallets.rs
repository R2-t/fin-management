use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use std::sync::Arc;

use super::create_wallet::ResponseBody;
use crate::{api::ApiError, auth::AuthPrincipal, AppState};

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> Result<impl IntoResponse, ApiError> {
    let wallets = state.wallet_service.list(principal.user_id).await?;
    let body: Vec<ResponseBody> = wallets.into_iter().map(ResponseBody::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}