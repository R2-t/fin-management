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
    application::inbound::repositories::{ItemInput, ItemRepository},
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
    pub name: String,
    pub unit_price: f64,
    pub quantity: i32,
    pub item_type: Option<String>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Path(params): Path<PathParams>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let input = ItemInput {
        name: body.name,
        unit_price: body.unit_price,
        quantity: body.quantity,
        item_type: body.item_type,
    };

    let item = state
        .item_repo
        .update(params.purchase_id, params.sequence, input)
        .await?;

    Ok((StatusCode::OK, Json(ResponseBody::from(item))).into_response())
}
