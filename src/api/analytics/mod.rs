use axum::{routing::get, Router};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

/// Both report endpoints take a half-open range; the check belongs in one place.
pub fn validate_range(
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<(), crate::api::ApiError> {
    if from >= to {
        return Err(crate::api::ApiError::BadRequest(
            "`from` must be earlier than `to`".into(),
        ));
    }
    Ok(())
}

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/trends", get(handlers::trends::handler))
        .route("/rolling", get(handlers::rolling::handler))
}
