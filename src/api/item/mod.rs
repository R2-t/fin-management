use axum::{
    routing::{delete, post, put},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(handlers::create_item::handler))
        .route(
            "/:purchase_id/:sequence",
            put(handlers::update_item::handler),
        )
        .route(
            "/:purchase_id/:sequence",
            delete(handlers::delete_item::handler),
        )
}
