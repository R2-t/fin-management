use axum::{
    routing::get,
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_forecasts::handler))
        .route("/budget/:budget_id/overspend", get(handlers::overspend::handler))
}
