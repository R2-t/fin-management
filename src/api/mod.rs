use axum::{response::IntoResponse, routing::get, Router};

pub mod transaction;
pub mod wallet;

pub fn setup_routes() -> Router {
    let transaction_routes = transaction::setup_transaction_routes();
    let wallet_routes = wallet::setup_wallet_routes();

    Router::new()
        .nest("/transaction", transaction_routes)
        .nest("/wallet", wallet_routes)
        .route("/health", get(check_health))
}

async fn check_health() -> impl IntoResponse {
    "pong".into_response()
}
