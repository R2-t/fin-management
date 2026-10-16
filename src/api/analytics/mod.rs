use axum::{routing::get, Router};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/trends", get(handlers::trends::handler))
        .route("/rolling", get(handlers::rolling::handler))
}
