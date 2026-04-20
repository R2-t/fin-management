use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError,
    application::purchase::commands::UpdatePurchaseCommand,
    AppState,
};

use super::PurchaseResponse;

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub user_id: Uuid,
    pub version: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cmd = UpdatePurchaseCommand {
        id,
        user_id: body.user_id,
        version: body.version,
        store: body.store,
        description: body.description,
        notes: body.notes,
        category_id: body.category_id,
    };

    let purchase = state.purchase_service.update(cmd).await?;

    Ok((StatusCode::OK, Json(PurchaseResponse::from(purchase))).into_response())
}
