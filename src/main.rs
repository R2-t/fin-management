use axum::Router;
use tower_http::trace::TraceLayer;

mod api;
mod setup;

#[tokio::main]
async fn main() {
    let routes = api::setup_routes();
    let app = Router::new()
        .nest("/fin-management", routes)
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
