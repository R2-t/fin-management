use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub user_id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
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
            created_at: w.created_at,
            updated_at: w.updated_at,
            deleted_at: w.deleted_at,
        }
    }
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let wallet = state.wallet_service.create(body.user_id, body.name).await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::from(wallet))).into_response())
}
