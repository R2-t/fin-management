use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_categories::handler))
        .route("/", post(handlers::create_category::handler))
}
