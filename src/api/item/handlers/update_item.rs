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
    application::inbound::repositories::ItemRepository,
    application::purchase::commands::UpdateItemCommand,
    auth::AuthPrincipal,
    AppState,
};

use super::create_item::ResponseBody;

#[derive(Debug, Deserialize)]
pub struct PathParams {
    pub purchase_id: Uuid,
    pub sequence: i32,
}

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub current_version: i32,
    pub name: Option<String>,
    pub unit_price: Option<i64>,
    pub quantity: Option<i32>,
    pub item_type: Option<String>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(params): Path<PathParams>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cmd = UpdateItemCommand {
        id: params.purchase_id,
        user_id: principal.user_id,
        current_version: body.current_version,
        sequence: params.sequence,
        name: body.name,
        unit_price: body.unit_price,
        quantity: body.quantity,
        item_type: body.item_type,
    };

    state.purchase_service.update_item(cmd).await?;

    let item = state
        .item_repo
        .get(params.purchase_id, params.sequence)
        .await?;

    Ok((StatusCode::OK, Json(ResponseBody::from(item))).into_response())
}