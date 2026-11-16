use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_rules::handler))
        .route("/", post(handlers::create_rule::handler))
        .route("/:id", delete(handlers::delete_rule::handler))
        .route("/:id/cancel", post(handlers::cancel_rule::handler))
}
