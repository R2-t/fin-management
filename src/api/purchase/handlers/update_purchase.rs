use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError,
    application::purchase::commands::UpdatePurchaseCommand,
    auth::AuthPrincipal,
    AppState,
};

use super::PurchaseResponse;

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub current_version: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
    pub merchant_id: Option<Uuid>,
    /// Decimal rate, used to complete an Unresolved rate. Converted at the boundary.
    #[serde(default)]
    pub exchange_rate: Option<f64>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cmd = UpdatePurchaseCommand {
        id,
        user_id: principal.user_id,
        current_version: body.current_version,
        store: body.store,
        description: body.description,
        notes: body.notes,
        category_id: body.category_id,
        merchant_id: body.merchant_id,
        exchange_rate: body.exchange_rate,
    };

    let purchase = state.purchase_service.update(cmd).await?;

    Ok((StatusCode::OK, Json(PurchaseResponse::from(purchase))).into_response())
}