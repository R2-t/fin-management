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
    application::outbound::analytics::ServiceTokenSource,
    application::purchase::commands::UpdatePurchaseCommand,
    auth::AuthPrincipal,
    repository::analytics_job::enqueue_suggestion,
    AppState,
};

use super::PurchaseResponse;

/// Records that the user's category differed from the recorded prediction (3.6).
///
/// Nothing is written when they agree, or when there was no prediction: feedback
/// means a disagreement, and a purchase the system never spoke about has no
/// disagreement to learn from.
async fn record_feedback(
    pool: &crate::PgPool,
    purchase_id: Uuid,
    corrected_category_id: i32,
) -> Result<bool, String> {
    let conn = pool.get().await.map_err(|e| e.to_string())?;
    let inserted = conn
        .execute(
            r#"INSERT INTO classification_feedback
                   (purchase_id, predicted_category_id, corrected_category_id, model_version)
               SELECT c.purchase_id, c.predicted_category_id, $2, c.model_version
                 FROM purchase_classification c
                WHERE c.purchase_id = $1
                  AND c.predicted_category_id IS NOT NULL
                  AND c.predicted_category_id <> $2
                  -- One correction per purchase per category: editing notes does
                  -- not re-record the same disagreement.
                  AND NOT EXISTS (
                      SELECT 1 FROM classification_feedback f
                       WHERE f.purchase_id = c.purchase_id
                         AND f.corrected_category_id = $2
                  )"#,
            &[&purchase_id, &corrected_category_id],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(inserted > 0)
}

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub current_version: i32,
    pub store: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub category_id: Option<i32>,
    pub merchant_id: Option<Uuid>,
    /// Correct the recorded date; moves the purchase between reporting buckets.
    #[serde(default)]
    pub purchase_date: Option<chrono::DateTime<chrono::Utc>>,
    /// Decimal rate, used to complete an Unresolved rate. Converted at the boundary.
    #[serde(default)]
    pub exchange_rate: Option<f64>,
}

pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Extension(pool): Extension<crate::PgPool>,
    Extension(auth): Extension<Option<Arc<dyn ServiceTokenSource>>>,
    Path(id): Path<Uuid>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    // An edit that could change what a suggestion would say (D77): the purchase
    // is still uncategorized and the edit touches the merchant or the descriptor
    // the predictor reads. Editing notes cannot change the answer, so it does not
    // queue work.
    let worth_resuggesting = body.merchant_id.is_some() || body.store.is_some();

    let cmd = UpdatePurchaseCommand {
        id,
        user_id: principal.user_id,
        current_version: body.current_version,
        store: body.store,
        description: body.description,
        notes: body.notes,
        category_id: body.category_id,
        merchant_id: body.merchant_id,
        purchase_date: body.purchase_date,
        exchange_rate: body.exchange_rate,
    };

    let purchase = state.purchase_service.update(cmd).await?;

    // The user changed the category of a purchase the system had suggested a
    // category for (3.6): that disagreement is the training signal, and it is
    // worth recording even though the user's choice wins (D62).
    //
    // The predicted category comes from the recorded classification row, not
    // from a fresh computation: what matters is what the system said at the
    // time, which is exactly what the user is reacting to.
    if let Some(chosen) = body.category_id {
        match record_feedback(&pool, purchase.id, chosen).await {
            Ok(true) => tracing::info!("recorded a category correction"),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "could not record the correction"),
        }
    }

    // Retries the failed-create case: a purchase whose job expired unclaimed
    // gets a fresh one when the user next touches it. Never fails the edit -- the
    // money is already recorded either way.
    if worth_resuggesting && purchase.category_id.is_none() {
        match enqueue_suggestion(
            &pool,
            auth.as_deref(),
            purchase.user_id,
            purchase.id,
            purchase.version,
        )
        .await
        {
            Ok(true) => tracing::info!("queued a category suggestion after an edit"),
            Ok(false) => {}
            Err(e) => tracing::warn!(error = %e, "could not queue a suggestion after an edit"),
        }
    }

    Ok((StatusCode::OK, Json(PurchaseResponse::from(purchase))).into_response())
}