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
    AppState,
};

use super::{ItemDto, PurchaseResponse};

/// How long a purchase recording may wait for a suggestion (D67).
///
/// The user is holding a receipt in a shop queue. Beyond this the answer is not
/// worth the wait, and the queue produces it a moment later.
const INLINE_BUDGET: std::time::Duration = std::time::Duration::from_millis(150);

/// Tries the sidecar inline, and falls back to the job queue.
///
/// Every failure path ends in the queue rather than in an error: a slow sidecar,
/// an unreachable one, a refused token and a missing credential all mean "not
/// now", and the purchase has already been recorded either way.
async fn suggest_inline_or_queue(
    state: &Arc<AppState>,
    pool: &crate::PgPool,
    auth: Option<&dyn ServiceTokenSource>,
    purchase: &crate::model::purchase::Purchase,
) -> Option<crate::application::outbound::analytics::ClassifyResult> {
    {
        let analytics = state.classify_backend.as_ref();
        let request = crate::application::outbound::analytics::ClassifyRequest {
            user_id: purchase.user_id,
            purchase_id: purchase.id,
        };
        match tokio::time::timeout(INLINE_BUDGET, analytics.classify(request)).await {
            Ok(Ok(Some(result))) if result.category_id.is_some() => {
                match apply_suggestion(pool, purchase, result.category_id, result.confidence).await
                {
                    // `false` means the row no longer wanted the suggestion --
                    // the user categorized it first. The response must not then
                    // claim a category the row does not hold, so the queue is
                    // consulted instead of returning the suggestion.
                    Ok(true) => return Some(result),
                    Ok(false) => {
                        tracing::info!("the purchase moved on; not applying the suggestion")
                    }
                    Err(e) => tracing::warn!(error = %e, "could not apply the inline suggestion"),
                }
            }
            Ok(Ok(_)) => {
                // The sidecar looked and had nothing honest to say. The queue
                // would say the same thing, so it is not queued.
                return None;
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "inline suggestion failed"),
            Err(_) => tracing::info!("inline suggestion exceeded its budget; queuing it"),
        }
    }

    match crate::repository::analytics_job::enqueue_suggestion(
        pool,
        auth,
        purchase.user_id,
        purchase.id,
        purchase.version,
    )
    .await
    {
        Ok(true) => tracing::info!("queued a category suggestion for the purchase"),
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "could not queue a category suggestion"),
    }
    None
}

/// Applies a suggestion: the application owns this write (D66).
///
/// Through the repository, so the version bumps, history records why, and the
/// outbox tells the projection to rebuild. An earlier version of this wrote the
/// column directly, and every projection kept showing the purchase
/// uncategorized.
async fn apply_suggestion(
    pool: &crate::PgPool,
    purchase: &crate::model::purchase::Purchase,
    category_id: Option<i32>,
    confidence: f64,
) -> Result<bool, String> {
    let Some(category_id) = category_id else {
        return Ok(false);
    };
    crate::repository::purchase::apply_suggested_category(
        pool,
        purchase.user_id,
        purchase.id,
        category_id,
        confidence,
        "merchant-history-v1",
        purchase.version,
    )
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

    // A purchase the user did not categorize gets a suggestion (D67): inline
    // first, bounded by a budget, and the queue as the fallback.
    //
    // Nothing here may fail the create. The money is recorded; telling someone
    // their purchase failed because a suggestion could not be computed would be
    // a lie about their ledger.
    let mut purchase = purchase;
    if purchase.category_id.is_none() {
        // The response carries the suggestion when the sidecar answered inside
        // the budget (D67): the user is looking at this answer, and telling them
        // "unclassified" while the row has a category would be a contradiction.
        if let Some(suggestion) =
            suggest_inline_or_queue(&state, &pool, auth.as_deref(), &purchase).await
        {
            purchase.category_id = suggestion.category_id;
            purchase.classification_status = "suggested".to_string();
            // The apply bumped the row's version. Reporting the old one would
            // hand the user a version that a PATCH immediately conflicts on.
            purchase.version += 1;
        }
    }

    Ok((StatusCode::CREATED, Json(PurchaseResponse::from(purchase))).into_response())
}
