use axum::Router;

mod api;

#[tokio::main]
async fn main() {
    let routes = api::setup_routes();
    let app = Router::new().nest("/fin-mang", routes);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
