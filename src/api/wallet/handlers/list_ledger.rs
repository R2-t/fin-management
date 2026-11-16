use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{api::ApiError, auth::AuthPrincipal, model::ledger::LedgerEntry, AppState};

#[derive(Debug, Deserialize)]
pub struct QueryParams {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    /// Signed minor units: credits positive, debits negative.
    pub amount_minor: i64,
    pub currency: String,
    pub kind: String,
    pub purchase_id: Option<Uuid>,
    pub transfer_id: Option<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<LedgerEntry> for ResponseBody {
    fn from(e: LedgerEntry) -> Self {
        Self {
            id: e.id,
            wallet_id: e.wallet_id,
            occurred_at: e.occurred_at,
            amount_minor: e.amount_minor,
            currency: e.currency,
            kind: e.kind.as_str().to_string(),
            purchase_id: e.purchase_id,
            transfer_id: e.transfer_id,
            created_at: e.created_at,
        }
    }
}

/// A Wallet's ledger entries, newest first.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(wallet_id): Path<Uuid>,
    Query(params): Query<QueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let entries = state
        .ledger_service
        .list(
            principal.user_id,
            wallet_id,
            params.limit,
            params.offset,
        )
        .await?;

    let body: Vec<ResponseBody> = entries.into_iter().map(ResponseBody::from).collect();
    Ok((StatusCode::OK, Json(body)).into_response())
}
