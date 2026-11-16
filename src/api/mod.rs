use axum::{
    http::StatusCode, middleware::from_fn, response::IntoResponse, routing::get, routing::post, Json,
    Router,
};
use serde_json::json;
use thiserror::Error;

use crate::application::error::ApplicationError;
use crate::auth::auth_middleware;

pub mod account_group;
pub mod analytics;
pub mod auth;
pub mod budget;
pub mod category;
pub mod forecast;
pub mod income;
pub mod item;
pub mod purchase;
pub mod recurring;
pub mod transfer;
pub mod user_profile;
pub mod wallet;

/// Route template + method of the matched request, attached to the response so
/// the outer `TraceLayer` (which runs before routing) can label metrics and
/// span names with low-cardinality values instead of raw paths.
#[derive(Clone, Debug)]
pub struct RouteLabel {
    pub method: String,
    pub route: String,
}

async fn route_label(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().to_string();
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "unmatched".to_string());
    let mut res = next.run(req).await;
    res.extensions_mut().insert(RouteLabel { method, route });
    res
}

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
        .nest("/account-group", account_group::setup_routes())
        .nest("/profile", user_profile::setup_routes())
        .nest("/analytics", analytics::setup_routes())
        .nest("/budget", budget::setup_routes())
        .nest("/forecast", forecast::setup_routes())
        .nest("/income", income::setup_routes())
        .nest("/recurring", recurring::setup_routes())
        .nest("/transfer", transfer::setup_routes())
        // Everything registered BEFORE this layer is protected; everything
        // after it is public. `/health` relies on that, and so do `/auth/login`
        // and `/auth/register` -- registering either of them on the wrong side
        // makes it either unreachable or unauthenticated, which is why
        // `tests/auth.rs` asserts both directions.
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .layer(from_fn(auth_middleware))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/health", get(check_health))
        .layer(from_fn(route_label))
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
            ApplicationError::Serialization(ref err) => {
                tracing::info!(
                    monotonic_counter.internal_errors_total = 1u64,
                    kind = "serialization"
                );
                tracing::error!(kind = "serialization", error = %err, "request failed with internal error");
                ApiError::Internal
            }
            ApplicationError::Internal(ref msg) => {
                tracing::info!(
                    monotonic_counter.internal_errors_total = 1u64,
                    kind = "internal"
                );
                tracing::error!(kind = "internal", error = %msg, "request failed with internal error");
                ApiError::Internal
            }
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
