use std::{sync::Arc, time::Duration};

use axum::http::Response;
use axum::{http::Request, Extension, Router};
use bb8_postgres::bb8::Pool;
use bb8_postgres::PostgresConnectionManager;
use config::get_service_name;
use config::otlpconfig::LoggerWrapper;
use tokio_postgres::NoTls;
use tower_http::trace::{DefaultOnRequest, TraceLayer};
use tracing::{field, info_span, Level, Span};

mod api;
mod config;

pub struct AppState {
    logger: LoggerWrapper,
    pg_pool: Pool<PostgresConnectionManager<NoTls>>,
}

#[tokio::main]
async fn main() {
    let config = config::Config::new();

    let (logger, pg_pool) = config.init();

    let app_state = Arc::new(AppState { logger, pg_pool });

    let app = Router::new()
        .nest(&format!("/{}", get_service_name()), api::setup_routes())
        .layer(Extension(app_state))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let host = request.headers().get("host").unwrap().to_str().unwrap().to_string();
                    info_span!("request", http.request.method = %&request.method(), http.request.headers = ?request.headers(), http.request.host = %host,http.request.url = %request.uri().path(), http.response.status = field::Empty, otel.name = %format!("Request  {} {}", request.method(), request.uri().path()))
                })
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(|response: &Response<_>, _latency: Duration, span: &Span| {
                    span.record("http.response.status", response.status().as_u16());
                }),
        );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
