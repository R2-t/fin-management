use axum::{
    extract::{State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError,
    application::outbound::analytics::ServiceTokenSource,
    application::purchase::commands::CreatePurchaseCommand,
    auth::AuthPrincipal,
    repository::analytics_job::enqueue_classify,
    AppState,
};

use super::{ItemDto, PurchaseResponse};

/// Mints the job's credential and enqueues it.
///
/// Without a service key there is nothing to mint, so the job is not written:
/// it could only fail terminally in the worker, and a row that cannot run is
/// worse than no row.
async fn enqueue_suggestion(
    pool: &crate::PgPool,
    auth: Option<&dyn ServiceTokenSource>,
    purchase: &crate::model::purchase::Purchase,
) -> Result<bool, String> {
    let Some(source) = auth else {
        return Err("no service key is configured".into());
    };
    let token = source
        .token_for(purchase.user_id)
        .await
        .map_err(|e| format!("minting failed: {e}"))?;
    enqueue_classify(pool, purchase.user_id, purchase.id, purchase.version, &token)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub wallet_id: Uuid,
    #[serde(default)]
    pub category_id: Option<i32>,
    #[serde(default)]
    pub merchant_id: Option<Uuid>,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub currency: String,
    #[serde(default)]
    pub exchange_rate_override: Option<f64>,
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    pub items: Vec<ItemDto>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Extension(pool): Extension<crate::PgPool>,
    Extension(auth): Extension<Option<Arc<dyn ServiceTokenSource>>>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cmd = CreatePurchaseCommand {
        user_id: principal.user_id,
        wallet_id: body.wallet_id,
        category_id: body.category_id,
        merchant_id: body.merchant_id,
        store: body.store,
        description: body.description,
        notes: body.notes,
        currency: body.currency,
        exchange_rate_override: body.exchange_rate_override,
        purchase_date: body.purchase_date,
        items: body.items.into_iter().map(Into::into).collect(),
    };

    let purchase = state.purchase_service.create(cmd).await?;

    // A purchase the user did not categorize becomes a suggestion job (D67).
    //
    // The enqueue must not fail the create: the purchase is recorded, and
    // telling the user their purchase failed because a background suggestion
    // could not be queued would be a lie about the money. A missing credential
    // or a database error is logged and the purchase stands.
    if purchase.category_id.is_none() {
        match enqueue_suggestion(&pool, auth.as_deref(), &purchase).await {
            Ok(true) => tracing::info!("queued a category suggestion for the purchase"),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "could not queue a category suggestion"),
        }
    }

    Ok((StatusCode::CREATED, Json(PurchaseResponse::from(purchase))).into_response())
}
