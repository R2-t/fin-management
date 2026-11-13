use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_budgets::handler))
        .route("/", post(handlers::create_budget::handler))
        .route("/:id", delete(handlers::delete_budget::handler))
        .route("/:id/status", get(handlers::get_budget_status::handler))
        .route("/:id/period", put(handlers::set_period_limit::handler))
}
