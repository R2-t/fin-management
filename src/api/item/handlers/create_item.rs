use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError,
    application::inbound::repositories::{ItemInput, ItemRepository},
    AppState,
};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub purchase_id: Uuid,
    pub name: String,
    pub unit_price: i64,
    pub quantity: i32,
    pub item_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub item_sequence: i32,
    pub purchase_id: Uuid,
    pub name: String,
    pub unit_price: i64,
    pub quantity: i32,
    pub item_type: Option<String>,
}

impl From<crate::model::item::Item> for ResponseBody {
    fn from(i: crate::model::item::Item) -> Self {
        Self {
            item_sequence: i.item_sequence,
            purchase_id: i.purchase_id,
            name: i.name,
            unit_price: i.unit_price,
            quantity: i.quantity,
            item_type: i.item_type,
        }
    }
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let input = ItemInput {
        name: body.name,
        unit_price: body.unit_price,
        quantity: body.quantity,
        item_type: body.item_type,
    };

    let item = state.item_repo.create(body.purchase_id, input).await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::from(item))).into_response())
}
