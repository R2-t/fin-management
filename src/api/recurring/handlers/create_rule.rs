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
    api::ApiError, application::currency::minor_unit,
    application::recurring::service::CreateRecurringRule, auth::AuthPrincipal,
    model::recurring::RecurrenceCadence, AppState,
};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub wallet_id: Uuid,
    /// The merchant it is paid to. Optional.
    #[serde(default)]
    pub merchant_id: Option<Uuid>,
    #[serde(default)]
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    /// ISO-4217 alpha code; defaults to the wallet's currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// 'weekly' | 'biweekly' | 'semimonthly' | 'monthly' | 'yearly'.
    pub cadence: String,
    /// The first expected date, as YYYY-MM-DD.
    pub starts_on: chrono::NaiveDate,
    #[serde(default)]
    pub ends_on: Option<chrono::NaiveDate>,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub wallet_id: Uuid,
    pub merchant_id: Option<Uuid>,
    pub category_id: Option<i32>,
    pub name: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_minor_unit: u8,
    pub cadence: String,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::model::recurring::RecurringRule> for ResponseBody {
    fn from(r: crate::model::recurring::RecurringRule) -> Self {
        Self {
            id: r.id,
            wallet_id: r.wallet_id,
            merchant_id: r.merchant_id,
            category_id: r.category_id,
            name: r.name,
            amount_minor: r.amount_minor,
            currency_minor_unit: minor_unit(&r.currency),
            currency: r.currency,
            cadence: r.cadence.as_str().to_string(),
            starts_on: r.starts_on,
            ends_on: r.ends_on,
            status: r.status.as_str().to_string(),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Declares a recurring commitment. Nothing is written to the ledger until the
/// user records the actual purchase.
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cadence = match body.cadence.to_ascii_lowercase().as_str() {
        "weekly" => RecurrenceCadence::Weekly,
        "biweekly" => RecurrenceCadence::Biweekly,
        "semimonthly" => RecurrenceCadence::Semimonthly,
        "monthly" => RecurrenceCadence::Monthly,
        "yearly" => RecurrenceCadence::Yearly,
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown cadence {other:?}: expected weekly, biweekly, semimonthly, monthly or yearly"
            )))
        }
    };

    let rule = state
        .recurring_rule_service
        .create(
            principal.user_id,
            CreateRecurringRule {
                wallet_id: body.wallet_id,
                merchant_id: body.merchant_id,
                category_id: body.category_id,
                name: body.name,
                amount_minor: body.amount_minor,
                currency: body.currency,
                cadence,
                starts_on: body.starts_on,
                ends_on: body.ends_on,
            },
        )
        .await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::from(rule))).into_response())
}
