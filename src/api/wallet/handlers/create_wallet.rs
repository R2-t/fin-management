use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub name: String,
    /// ISO-4217 alpha code; defaults to the user's reporting currency.
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub currency: String,
    /// Decimal places for `currency`, so clients render minor units correctly.
    pub currency_minor_unit: u8,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<crate::model::wallet::Wallet> for ResponseBody {
    fn from(w: crate::model::wallet::Wallet) -> Self {
        Self {
            id: w.id,
            user_id: w.user_id,
            name: w.name,
            currency_minor_unit: crate::application::currency::minor_unit(&w.currency),
            currency: w.currency,
            created_at: w.created_at,
            updated_at: w.updated_at,
            deleted_at: w.deleted_at,
        }
    }
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let wallet = state
        .wallet_service
        .create(principal.user_id, body.name, body.currency)
        .await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::from(wallet))).into_response())
}
