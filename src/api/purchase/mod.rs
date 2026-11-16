use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_purchases::handler))
        .route("/:id", get(handlers::get_purchase_details::handler))
        .route(
            "/purchase_with_items",
            post(handlers::create_purchase_with_items::handler),
        )
        .route("/:id", put(handlers::update_purchase::handler))
        .route("/:id", delete(handlers::delete_purchase::handler))
}
