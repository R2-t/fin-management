use axum::{response::IntoResponse, routing::get, Router};

pub mod transaction;

pub fn setup_routes() -> Router {
    let transaction_routes = transaction::setup_transaction_routes();

    Router::new()
        .nest("/transaction", transaction_routes)
        .route("/health", get(check_health))
}

async fn check_health() -> impl IntoResponse {
    "pong".into_response()
}
