use axum::{
    routing::{delete, get, post, put},
    Router,
};

pub mod handlers;

pub fn setup_wallet_routes() -> Router {
    Router::new()
        .route("", get(handlers::list_wallets::handler))
        .route("/:id", get(handlers::get_wallet::handler))
        .route("", post(handlers::create_wallet::handler))
        .route("/:id", put(handlers::update_wallet::handler))
        .route("/:id", delete(handlers::delete_wallet::handler))
}
