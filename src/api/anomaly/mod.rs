use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_anomalies::handler))
        .route("/:id/dismiss", post(handlers::dismiss_anomaly::handler))
        .route("/:id/undismiss", post(handlers::undismiss_anomaly::handler))
}
