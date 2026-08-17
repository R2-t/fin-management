use std::{sync::Arc, time::Duration};

use axum::http::Response;
use axum::{http::Request, Router};
use config::get_service_name;
use config::otlpconfig::LoggerWrapper;
use tower_http::trace::{DefaultOnRequest, TraceLayer};
use tracing::{field, info_span, Level, Span};

pub mod api;
pub mod application;
pub mod auth;
pub mod config;
pub mod model;
pub mod repository;

use application::{
    category::service::CategoryService, outbound::exchange_rate::MockExchangeRateProvider,
    purchase::service::PurchaseService, wallet::service::WalletService,
};
use auth::{
    paseto::PasetoVerifier,
    verifier::TokenVerifier,
};
use repository::{
    category::PgCategoryRepo, item::PgItemRepo, purchase::PgPurchaseRepo,
    rate_cache::PgRateCacheRepo, wallet::PgWalletRepo,
};

pub struct AppState {
    pub logger: LoggerWrapper,
    pub purchase_service: Arc<
        PurchaseService<PgPurchaseRepo, PgRateCacheRepo, PgWalletRepo, MockExchangeRateProvider>,
    >,
    pub wallet_service: Arc<WalletService<PgWalletRepo>>,
    pub category_service: Arc<CategoryService<PgCategoryRepo>>,
    pub item_repo: Arc<PgItemRepo>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let config = config::Config::new();

    let (logger, pg_pool) = config.init();

    let purchase_repo = PgPurchaseRepo::new(pg_pool.clone());
    let item_repo = PgItemRepo::new(pg_pool.clone());
    let wallet_repo = PgWalletRepo::new(pg_pool.clone());
    let category_repo = PgCategoryRepo::new(pg_pool.clone());
    let rate_cache_repo = PgRateCacheRepo::new(pg_pool.clone());

    // Mock FX provider until a real one is wired.
    let fx_provider = MockExchangeRateProvider::new(std::collections::HashMap::new());

    let purchase_service = Arc::new(PurchaseService::new(
        purchase_repo,
        rate_cache_repo,
        wallet_repo,
        fx_provider,
    ));
    let wallet_service = Arc::new(WalletService::new(PgWalletRepo::new(pg_pool.clone())));
    let category_service = Arc::new(CategoryService::new(category_repo));
    let item_repo = Arc::new(item_repo);

    let app_state = Arc::new(AppState {
        logger,
        purchase_service,
        wallet_service,
        category_service,
        item_repo,
    });

    let token_verifier = build_token_verifier();

    let app = Router::new()
        .nest(&format!("/{}", get_service_name()), api::setup_routes())
        .with_state(app_state)
        .layer(axum::extract::Extension(token_verifier))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let host = request
                        .headers()
                        .get("host")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown")
                        .to_string();
                    info_span!(
                        "request",
                        http.request.method = %request.method(),
                        http.request.host = %host,
                        http.request.url = %request.uri().path(),
                        http.response.status = field::Empty,
                        otel.name = %format!("Request {} {}", request.method(), request.uri().path())
                    )
                })
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(|response: &Response<_>, _latency: Duration, span: &Span| {
                    span.record("http.response.status", response.status().as_u16());
                }),
        );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();

    tracing::info!("listening on 0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}

/// Build the token verifier used by the auth middleware.
///
/// TODO(auth): Load the key set from the external key service (which owns key
/// material and performs rotation) instead of a static in-memory map.
fn build_token_verifier() -> Arc<dyn TokenVerifier> {
    let key_provider = Arc::new(auth::key_provider::StaticKeyProvider::new(
        std::collections::HashMap::new(),
    ));
    Arc::new(PasetoVerifier::new(key_provider))
}
