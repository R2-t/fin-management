use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_transfers::handler))
        .route("/", post(handlers::create_transfer::handler))
        .route("/:id", delete(handlers::delete_transfer::handler))
}
