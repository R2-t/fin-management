//! Library surface of the financial-management service.
//!
//! The binary in `main.rs` and the integration tests in `tests/` share this
//! crate so that both exercise the *same* application wiring: `build_router`
//! is the single way to assemble the HTTP application.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::Response;
use axum::{http::Request, Router};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::{DefaultOnRequest, TraceLayer};
use tracing::{field, info_span, Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub mod api;
pub mod application;
pub mod auth;
pub mod config;
pub mod model;
pub mod repository;

use application::{
    category::service::CategoryService, outbound::exchange_rate::MockExchangeRateProvider,
    purchase::service::PurchaseService, user_profile::service::UserProfileService,
    wallet::service::WalletService,
};
use auth::{paseto::PasetoVerifier, verifier::TokenVerifier};
use repository::{
    analytics::PgAnalyticsRepo, category::PgCategoryRepo, item::PgItemRepo,
    purchase::PgPurchaseRepo, rate_cache::PgRateCacheRepo, user_profile::PgUserProfileRepo,
    wallet::PgWalletRepo, PgPool,
};

/// Adapter so the OTel propagator can read W3C headers from an axum request.
struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

/// Converts a handler panic into a logged, counted 500 response.
fn handle_panic(err: Box<dyn std::any::Any + Send + 'static>) -> Response<axum::body::Body> {
    let detail = err
        .downcast_ref::<String>()
        .map(|s| s.as_str())
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic payload");
    tracing::info!(monotonic_counter.panics_total = 1u64);
    tracing::error!(panic = %detail, "handler panicked");
    Response::builder()
        .status(500)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            r#"{"error":"Internal server error"}"#,
        ))
        .expect("static response is valid")
}

pub struct AppState {
    pub purchase_service: Arc<
        PurchaseService<
            PgPurchaseRepo,
            PgItemRepo,
            PgRateCacheRepo,
            PgWalletRepo,
            MockExchangeRateProvider,
        >,
    >,
    pub wallet_service: Arc<WalletService<PgWalletRepo>>,
    pub category_service: Arc<CategoryService<PgCategoryRepo>>,
    pub user_profile_service: Arc<UserProfileService<PgUserProfileRepo>>,
    pub trends_service:
        Arc<application::trends::service::TrendsService<repository::analytics::PgAnalyticsRepo>>,
    pub item_repo: Arc<PgItemRepo>,
}

/// Builds the shared application state from a connection pool.
pub fn build_app_state(pg_pool: PgPool) -> Arc<AppState> {
    let purchase_repo = PgPurchaseRepo::new(pg_pool.clone());
    let item_repo = Arc::new(PgItemRepo::new(pg_pool.clone()));
    let wallet_repo = PgWalletRepo::new(pg_pool.clone());
    let category_repo = PgCategoryRepo::new(pg_pool.clone());
    let rate_cache_repo = PgRateCacheRepo::new(pg_pool.clone());

    // Mock FX provider until a real one is wired.
    let fx_provider = MockExchangeRateProvider::new(HashMap::new());

    let purchase_service = Arc::new(PurchaseService::new(
        purchase_repo,
        item_repo.as_ref().clone(),
        rate_cache_repo,
        wallet_repo,
        fx_provider,
    ));
    let wallet_service = Arc::new(WalletService::new(PgWalletRepo::new(pg_pool.clone())));
    let category_service = Arc::new(CategoryService::new(category_repo));
    let user_profile_service = Arc::new(UserProfileService::new(PgUserProfileRepo::new(
        pg_pool.clone(),
    )));

    let trends_service = Arc::new(application::trends::service::TrendsService::new(
        PgAnalyticsRepo::new(pg_pool.clone()),
    ));

    Arc::new(AppState {
        purchase_service,
        wallet_service,
        category_service,
        user_profile_service,
        trends_service,
        item_repo,
    })
}

/// The single way to assemble the HTTP application.
///
/// Used by the running server and by the integration tests, so both exercise
/// identical wiring (routes, auth middleware, panic handling, tracing layer).
pub fn build_router(pg_pool: &PgPool, verifier: Arc<dyn TokenVerifier>) -> Router {
    let app_state = build_app_state(pg_pool.clone());

    Router::new()
        .nest(
            &format!("/{}", config::get_service_name()),
            api::setup_routes(),
        )
        .with_state(app_state)
        .layer(axum::extract::Extension(verifier))
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let span = info_span!(
                        "request",
                        http.request.method = %request.method(),
                        http.request.url = %request.uri().path(),
                        http.route = field::Empty,
                        http.response.status = field::Empty,
                        otel.name = %request.method(),
                        otel.status_code = field::Empty,
                    );
                    // Continue an upstream trace if a `traceparent` header is present.
                    let parent = opentelemetry::global::get_text_map_propagator(|p| {
                        p.extract(&HeaderExtractor(request.headers()))
                    });
                    span.set_parent(parent);
                    span
                })
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(|response: &Response<_>, latency: Duration, span: &Span| {
                    let status = response.status().as_u16();
                    let (method, route) = response
                        .extensions()
                        .get::<api::RouteLabel>()
                        .map(|l| (l.method.clone(), l.route.clone()))
                        .unwrap_or_else(|| ("unknown".into(), "unmatched".into()));

                    span.record("http.response.status", status);
                    span.record("http.route", route.as_str());
                    span.record("otel.name", format!("{method} {route}").as_str());
                    if status >= 500 {
                        span.record("otel.status_code", "ERROR");
                    }

                    // RED metrics, labelled by route template (low cardinality).
                    tracing::info!(
                        histogram.http_request_duration_ms = latency.as_millis() as f64,
                        http.route = %route,
                        http.request.method = %method,
                        http.response.status = status,
                    );
                    tracing::info!(
                        monotonic_counter.http_requests_total = 1u64,
                        http.route = %route,
                        http.request.method = %method,
                        http.response.status = status,
                    );
                    if status >= 500 {
                        tracing::warn!(
                            monotonic_counter.http_5xx_total = 1u64,
                            http.route = %route,
                            http.request.method = %method,
                            http.response.status = status,
                        );
                    }
                }),
        )
}

/// Opens a pool from a connection string without initialising telemetry.
///
/// The server path uses `config::Config::init`, which also installs tracing and
/// metrics. Tests must not do that (the global subscriber can only be set once),
/// so they build their pool through this function.
pub async fn connect_pool(dsn: &str) -> Result<PgPool, config::postgresconfig::PoolError> {
    config::postgresconfig::pool_from_dsn(dsn).await
}

/// Build the token verifier used by the auth middleware.
///
/// Set `AUTH_SKIP_VERIFY=1` to bypass token verification entirely (local
/// testing only): every request is authenticated as `AUTH_DEFAULT_USER_ID`
/// (or a fixed well-known user when unset). Without that flag, the PASETO
/// verifier is used with the static in-memory key map.
///
/// TODO(auth): Load the key set from the external key service (which owns key
/// material and performs rotation) instead of a static in-memory map.
pub fn build_token_verifier() -> Arc<dyn TokenVerifier> {
    if std::env::var("AUTH_SKIP_VERIFY").is_ok_and(|v| v == "1") {
        let default_user = std::env::var("AUTH_DEFAULT_USER_ID")
            .ok()
            .and_then(|v| v.parse().ok());
        return Arc::new(auth::verifier::StaticUserVerifier::new(default_user));
    }

    let key_provider = Arc::new(auth::key_provider::StaticKeyProvider::new(HashMap::new()));
    Arc::new(PasetoVerifier::new(key_provider))
}

/// Handle for the analytics projection, exposed so maintenance commands and
/// tests can drive projection explicitly instead of waiting for the loop.
pub fn analytics_repo(pool: &PgPool) -> PgAnalyticsRepo {
    PgAnalyticsRepo::new(pool.clone())
}

/// Runs the analytics projection once, synchronously, to completion.
pub async fn drain_projection<R: application::inbound::repositories::AnalyticsRepository + Send + Sync + 'static>(
    repo: R,
) -> Result<usize, application::error::ApplicationError> {
    application::analytics::projector::FactProjector::new(repo)
        .drain()
        .await
}
