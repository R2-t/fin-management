#![allow(async_fn_in_trait)]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::application::error::ApplicationError;

#[derive(Debug, Error)]
pub enum RateError {
    #[error("Exchange rate provider unavailable")]
    ProviderUnavailable,
    #[error("No rate available for currency pair {from}/{to}")]
    PairNotFound { from: String, to: String },
    #[error("Invalid rate returned for currency pair {from}/{to}")]
    InvalidRate { from: String, to: String },
}

pub struct RateRequest {
    pub from: String,
    pub to: String,
    pub at: DateTime<Utc>,
}

pub struct RateResult {
    pub rate: f64,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
}

pub trait ExchangeRateProvider: Send + Sync {
    async fn fetch_rate(&self, req: RateRequest) -> Result<RateResult, RateError>;
}

/// Pluggable mock provider used in the monolith until a real provider is wired.
#[derive(Debug, Default)]
pub struct MockExchangeRateProvider {
    rates: Arc<HashMap<(String, String), f64>>,
}

impl MockExchangeRateProvider {
    pub fn new(rates: HashMap<(String, String), f64>) -> Self {
        Self {
            rates: Arc::new(rates),
        }
    }
}

impl ExchangeRateProvider for MockExchangeRateProvider {
    async fn fetch_rate(&self, req: RateRequest) -> Result<RateResult, RateError> {
        if req.from == req.to {
            return Ok(RateResult {
                rate: 1.0,
                source: "api".to_string(),
                fetched_at: Utc::now(),
            });
        }

        let key = (req.from.to_uppercase(), req.to.to_uppercase());
        let rate = self
            .rates
            .get(&key)
            .copied()
            .ok_or(RateError::PairNotFound {
                from: req.from,
                to: req.to,
            })?;

        if rate <= 0.0 {
            return Err(RateError::InvalidRate {
                from: key.0,
                to: key.1,
            });
        }

        Ok(RateResult {
            rate,
            source: "api".to_string(),
            fetched_at: Utc::now(),
        })
    }
}

impl From<RateError> for ApplicationError {
    fn from(e: RateError) -> Self {
        ApplicationError::Internal(format!("exchange rate: {e}"))
    }
}
