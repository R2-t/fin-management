use axum::{
    http::StatusCode, middleware::from_fn, response::IntoResponse, routing::get, Json, Router,
};
use serde_json::json;
use thiserror::Error;

use crate::application::error::ApplicationError;
use crate::auth::auth_middleware;

pub mod category;
pub mod item;
pub mod purchase;
pub mod wallet;

pub fn setup_routes() -> Router<std::sync::Arc<crate::AppState>> {
    let purchase_routes = purchase::setup_routes();
    let wallet_routes = wallet::setup_routes();
    let item_routes = item::setup_routes();
    let category_routes = category::setup_routes();

    Router::new()
        .nest("/purchase", purchase_routes)
        .nest("/wallet", wallet_routes)
        .nest("/item", item_routes)
        .nest("/category", category_routes)
        .layer(from_fn(auth_middleware))
        .route("/health", get(check_health))
}

async fn check_health() -> impl IntoResponse {
    (StatusCode::OK, "pong").into_response()
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("{0}")]
    BadRequest(String),
    #[error("Internal server error")]
    Internal,
}

impl From<ApplicationError> for ApiError {
    fn from(e: ApplicationError) -> Self {
        match e {
            ApplicationError::NotFound(msg) => ApiError::NotFound(msg),
            ApplicationError::Conflict(msg) => ApiError::Conflict(msg),
            ApplicationError::Unauthorized => ApiError::Unauthorized,
            ApplicationError::Validation(msg) => ApiError::BadRequest(msg),
            ApplicationError::Serialization(_) => ApiError::Internal,
            ApplicationError::Internal(_) => ApiError::Internal,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, body) = match &self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".into()),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".into(),
            ),
        };
        (status, Json(json!({ "error": body }))).into_response()
    }
}
