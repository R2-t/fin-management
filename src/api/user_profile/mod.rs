use axum::{
    routing::{get, put},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::get_profile::handler))
        .route("/", put(handlers::update_profile::handler))
}
