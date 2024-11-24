use axum::{
    routing::{delete, get, post, put},
    Router,
};

pub mod handlers;

pub fn setup_transaction_routes() -> Router {
    Router::new()
        .route("/", get(handlers::list::handler))
        .route("/:id", get(handlers::get::handler))
        .route("/", post(handlers::create::handler))
        .route("/:id", put(handlers::update::handler))
        .route("/:id", delete(handlers::delete::handler))
}
