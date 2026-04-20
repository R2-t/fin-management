use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_wallets::handler))
        .route("/", post(handlers::create_wallet::handler))
        .route("/:id", put(handlers::update_wallet::handler))
        .route("/:id", delete(handlers::delete_wallet::handler))
}
