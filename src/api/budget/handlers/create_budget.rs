use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    api::ApiError, application::budget::service::CreateBudget,
    application::currency::minor_unit, auth::AuthPrincipal, model::budget::BudgetCadence, AppState,
};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub wallet_id: Uuid,
    /// Omit to cover the whole wallet.
    #[serde(default)]
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    /// 'weekly' | 'monthly' | 'yearly'.
    pub cadence: String,
    /// The day the period starts: 1 for a calendar month, 15 for a mid-month
    /// payday, -1 for a period ending on the last day of the month.
    #[serde(default = "default_reset_day")]
    pub reset_day: i16,
    /// The first period's start date, as YYYY-MM-DD.
    pub starts_on: chrono::NaiveDate,
    #[serde(default)]
    pub ends_on: Option<chrono::NaiveDate>,
    /// Whether an overspend carries into the next period.
    #[serde(default = "default_true")]
    pub carry_overspend: bool,
}

fn default_reset_day() -> i16 {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_minor_unit: u8,
    pub cadence: String,
    pub reset_day: i16,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    pub carry_overspend: bool,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl ResponseBody {
    /// A Budget's currency is its Wallet's, so the caller supplies it. It is not
    /// a field of the Budget, and a default here would report the wrong
    /// currency for every non-USD wallet.
    pub(super) fn new(b: crate::model::budget::Budget, currency: String) -> Self {
        Self {
            id: b.id,
            wallet_id: b.wallet_id,
            category_id: b.category_id,
            name: b.name,
            amount_minor: b.amount_minor,
            currency_minor_unit: minor_unit(&currency),
            currency,
            cadence: b.cadence.as_str().to_string(),
            reset_day: b.reset_day,
            starts_on: b.starts_on,
            ends_on: b.ends_on,
            carry_overspend: b.carry_overspend,
            active: b.active,
            created_at: b.created_at,
            updated_at: b.updated_at,
        }
    }
}

/// Declares a budget: a spending limit for one category within one wallet,
/// over a budget period.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cadence = match body.cadence.to_ascii_lowercase().as_str() {
        "weekly" => BudgetCadence::Weekly,
        "monthly" => BudgetCadence::Monthly,
        "yearly" => BudgetCadence::Yearly,
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown cadence {other:?}: expected weekly, monthly or yearly"
            )))
        }
    };

    let budget = state
        .budget_service
        .create(
            principal.user_id,
            CreateBudget {
                wallet_id: body.wallet_id,
                category_id: body.category_id,
                name: body.name,
                amount_minor: body.amount_minor,
                cadence,
                reset_day: body.reset_day,
                starts_on: body.starts_on,
                ends_on: body.ends_on,
                carry_overspend: body.carry_overspend,
            },
        )
        .await?;

    let wallet = state
        .wallet_service
        .get(principal.user_id, budget.wallet_id)
        .await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::new(budget, wallet.currency))).into_response())
}
