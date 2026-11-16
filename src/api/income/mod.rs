use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_income_schedules::handler))
        .route("/", post(handlers::create_income_schedule::handler))
        .route("/:id", delete(handlers::delete_income_schedule::handler))
        .route("/:id/receive", post(handlers::receive_income::handler))
}
