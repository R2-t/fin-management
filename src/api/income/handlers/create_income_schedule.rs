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
    api::ApiError, application::currency::minor_unit, application::income::service::{
        CreateAllocation, CreateIncomeSchedule,
    },
    auth::AuthPrincipal, model::income::IncomeCadence, AppState,
};

#[derive(Debug, Deserialize)]
pub struct RequestBody {
    pub name: String,
    /// The whole amount, in minor units.
    pub amount_minor: i64,
    /// ISO-4217 alpha code; defaults to the default currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// 'weekly' | 'biweekly' | 'semimonthly' | 'monthly'.
    pub cadence: String,
    /// The first expected date, as YYYY-MM-DD.
    pub starts_on: chrono::NaiveDate,
    #[serde(default)]
    pub ends_on: Option<chrono::NaiveDate>,
    /// Where the money goes. Must sum to `amount_minor`.
    pub allocations: Vec<AllocationBody>,
}

#[derive(Debug, Deserialize)]
pub struct AllocationBody {
    pub wallet_id: Uuid,
    pub amount_minor: i64,
}

#[derive(Debug, Serialize)]
pub struct AllocationResponse {
    pub wallet_id: Uuid,
    pub amount_minor: i64,
}

#[derive(Debug, Serialize)]
pub struct ResponseBody {
    pub id: Uuid,
    pub name: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_minor_unit: u8,
    pub cadence: String,
    pub starts_on: chrono::NaiveDate,
    pub ends_on: Option<chrono::NaiveDate>,
    pub active: bool,
    pub allocations: Vec<AllocationResponse>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<crate::model::income::IncomeSchedule> for ResponseBody {
    fn from(s: crate::model::income::IncomeSchedule) -> Self {
        Self {
            id: s.id,
            name: s.name,
            amount_minor: s.amount_minor,
            currency_minor_unit: minor_unit(&s.currency),
            currency: s.currency,
            cadence: s.cadence.as_str().to_string(),
            starts_on: s.starts_on,
            ends_on: s.ends_on,
            active: s.active,
            allocations: s
                .allocations
                .into_iter()
                .map(|a| AllocationResponse {
                    wallet_id: a.wallet_id,
                    amount_minor: a.amount_minor,
                })
                .collect(),
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// Declares an income expectation: money expected on a cadence. It is not
/// money until an occurrence is received (ADR-0008).
pub async fn handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<RequestBody>,
) -> Result<impl IntoResponse, ApiError> {
    let cadence = match body.cadence.to_ascii_lowercase().as_str() {
        "weekly" => IncomeCadence::Weekly,
        "biweekly" => IncomeCadence::Biweekly,
        "semimonthly" => IncomeCadence::Semimonthly,
        "monthly" => IncomeCadence::Monthly,
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown cadence {other:?}: expected weekly, biweekly, semimonthly or monthly"
            )))
        }
    };

    let schedule = state
        .income_service
        .create(
            principal.user_id,
            CreateIncomeSchedule {
                name: body.name,
                amount_minor: body.amount_minor,
                currency: body.currency,
                cadence,
                starts_on: body.starts_on,
                ends_on: body.ends_on,
                allocations: body
                    .allocations
                    .into_iter()
                    .map(|a| CreateAllocation {
                        wallet_id: a.wallet_id,
                        amount_minor: a.amount_minor,
                    })
                    .collect(),
            },
        )
        .await?;

    Ok((StatusCode::CREATED, Json(ResponseBody::from(schedule))).into_response())
}
