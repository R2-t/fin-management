use axum::{
    routing::{delete, get, patch, put},
    Router,
};
use std::sync::Arc;

use crate::AppState;

pub mod handlers;

pub fn setup_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(handlers::list_groups::handler).post(handlers::create_group::handler))
        .route("/:id", patch(handlers::rename_group::handler))
        .route("/:id", delete(handlers::delete_group::handler))
        // Assigning a wallet is a wallet concern that names a group, so it lives
        // on the wallet id and takes the group in the body.
        .route("/wallet/:wallet_id", put(handlers::assign_wallet::handler))
}
