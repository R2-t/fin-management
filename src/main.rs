use std::{sync::Arc, time::Duration};

use axum::{Extension, Router};
use opentelemetry_sdk::logs::Logger;
use tower_http::trace::TraceLayer;

mod api;
mod config;

pub struct AppState {
    logger: Logger,
}

#[tokio::main]
async fn main() {
    let otlpconfig = config::otlpconfig::Config::new(
        "http://localhost:4317".to_string(),
        Duration::from_secs(5),
        "fin-management".to_string(),
    );

    let config = config::Config { otlpconfig };

    let logger = config.init();

    let app_state = Arc::new(AppState { logger });

    let app = Router::new()
        .nest("/fin-management", api::setup_routes())
        .layer(Extension(app_state))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
