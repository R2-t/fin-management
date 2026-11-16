use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    api::ApiError, application::currency::minor_unit, application::transfer::service::CreateTransfer,
    auth::AuthPrincipal, AppState,
};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub from_wallet_id: uuid::Uuid,
    pub to_wallet_id: uuid::Uuid,
    /// Minor units. Must be greater than zero.
    pub amount_minor: i64,
    /// ISO-4217 alpha code; defaults to the wallets' currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// When the transfer happened; defaults to now.
    #[serde(default)]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: uuid::Uuid,
    pub from_wallet_id: uuid::Uuid,
    pub to_wallet_id: uuid::Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_minor_unit: u8,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::model::transfer::Transfer> for ResponseBody {
    fn from(t: crate::model::transfer::Transfer) -> Self {
        Self {
            id: t.id,
            from_wallet_id: t.from_wallet_id,
            to_wallet_id: t.to_wallet_id,
            occurred_at: t.occurred_at,
            amount_minor: t.amount_minor,
            currency_minor_unit: minor_unit(&t.currency),
            currency: t.currency,
            description: t.description,
            created_at: t.created_at,
        }
    }
}

/// Records a transfer between two of the user's Wallets.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let transfer = state
        .transfer_service
        .create(
            principal.user_id,
            CreateTransfer {
                from_wallet_id: body.from_wallet_id,
                to_wallet_id: body.to_wallet_id,
                amount_minor: body.amount_minor,
                currency: body.currency,
                occurred_at: body.occurred_at,
                description: body.description,
            },
        )
        .await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::from(transfer))).into_response())
}
