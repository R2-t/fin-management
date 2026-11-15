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
    budget::service::BudgetService, burn_rate::service::BurnRateService,
    category::service::CategoryService, forecast::service::ForecastService,
    income::service::IncomeService, ledger::service::LedgerService,
    outbound::exchange_rate::MockExchangeRateProvider,
    outbound::http_analytics::NoAnalytics,
    purchase::service::PurchaseService, recurring::service::RecurringRuleService,
    runway::service::RunwayService, transfer::service::TransferService,
    user_profile::service::UserProfileService, wallet::service::WalletService,
};
use auth::{paseto::PasetoVerifier, verifier::TokenVerifier};
use repository::{
    analytics::PgAnalyticsRepo, budget::PgBudgetRepo, category::PgCategoryRepo,
    forecast::PgForecastRepo, income::PgIncomeRepo, item::PgItemRepo,
    ledger::PgLedgerRepo, purchase::PgPurchaseRepo, rate_cache::PgRateCacheRepo,
    recurring_rule::PgRecurringRuleRepo, transfer::PgTransferRepo,
    user_profile::PgUserProfileRepo, wallet::PgWalletRepo, PgPool,
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
    pub ledger_service: Arc<LedgerService<PgLedgerRepo>>,
    pub transfer_service: Arc<TransferService<PgTransferRepo, PgWalletRepo>>,
    pub income_service: Arc<IncomeService<PgIncomeRepo, PgWalletRepo>>,
    pub recurring_rule_service: Arc<RecurringRuleService<PgRecurringRuleRepo, PgWalletRepo>>,
    pub burn_rate_service: Arc<BurnRateService<PgAnalyticsRepo>>,
    pub runway_service: Arc<
        RunwayService<PgAnalyticsRepo, PgLedgerRepo, Arc<dyn crate::application::outbound::analytics::SpendAnalytics>>,
    >,
    pub budget_service: Arc<BudgetService<PgBudgetRepo, PgWalletRepo, PgAnalyticsRepo>>,
    pub forecast_service: Arc<
        ForecastService<PgForecastRepo, PgBudgetRepo, PgWalletRepo, PgAnalyticsRepo>,
    >,
    pub category_service: Arc<CategoryService<PgCategoryRepo>>,
    pub user_profile_service: Arc<UserProfileService<PgUserProfileRepo>>,
    pub trends_service:
        Arc<application::trends::service::TrendsService<repository::analytics::PgAnalyticsRepo>>,
    pub item_repo: Arc<PgItemRepo>,
    /// The sidecar, for calls a request is willing to wait 150ms for (D67).
    pub classify_backend: Arc<dyn application::outbound::analytics::SpendAnalytics>,
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
    let ledger_service = Arc::new(LedgerService::new(PgLedgerRepo::new(pg_pool.clone())));
    let transfer_service = Arc::new(TransferService::new(
        PgTransferRepo::new(pg_pool.clone()),
        PgWalletRepo::new(pg_pool.clone()),
    ));
    let income_service = Arc::new(IncomeService::new(
        PgIncomeRepo::new(pg_pool.clone()),
        PgWalletRepo::new(pg_pool.clone()),
    ));
    let recurring_rule_service = Arc::new(RecurringRuleService::new(
        PgRecurringRuleRepo::new(pg_pool.clone()),
        PgWalletRepo::new(pg_pool.clone()),
    ));
    let burn_rate_service = Arc::new(BurnRateService::new(PgAnalyticsRepo::new(
        pg_pool.clone(),
    )));
    // The sidecar when ANALYTICS_URL names one, otherwise `NoAnalytics`: the
    // deterministic projection is then the answer, and the assumptions say so
    // rather than inventing bands.
    let runway_service = Arc::new(RunwayService::new(
        PgAnalyticsRepo::new(pg_pool.clone()),
        PgLedgerRepo::new(pg_pool.clone()),
        analytics_backend(),
    ));
    let budget_service = Arc::new(BudgetService::new(
        PgBudgetRepo::new(pg_pool.clone()),
        PgWalletRepo::new(pg_pool.clone()),
        PgAnalyticsRepo::new(pg_pool.clone()),
    ));
    let forecast_service = Arc::new(ForecastService::new(
        PgForecastRepo::new(pg_pool.clone()),
        PgBudgetRepo::new(pg_pool.clone()),
        PgWalletRepo::new(pg_pool.clone()),
        PgAnalyticsRepo::new(pg_pool.clone()),
    ));
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
        ledger_service,
        transfer_service,
        income_service,
        recurring_rule_service,
        burn_rate_service,
        runway_service,
        budget_service,
        forecast_service,
        category_service,
        user_profile_service,
        trends_service,
        item_repo,
        classify_backend: analytics_backend(),
    })
}

/// The single way to assemble the HTTP application.
///
/// Used by the running server and by the integration tests, so both exercise
/// identical wiring (routes, auth middleware, panic handling, tracing layer).
pub fn build_router(pg_pool: &PgPool, verifier: Arc<dyn TokenVerifier>) -> Router {
    build_router_with_auth(pg_pool, verifier, config::auth::AuthConfig::from_env())
}

/// `build_router`, with the auth settings supplied rather than read from the
/// environment, so a test can pin the origin and the limits.
pub fn build_router_with_auth(
    pg_pool: &PgPool,
    verifier: Arc<dyn TokenVerifier>,
    auth_config: config::auth::AuthConfig,
) -> Router {
    let app_state = build_app_state(pg_pool.clone());

    Router::new()
        .nest(
            &format!("/{}", config::get_service_name()),
            api::setup_routes(),
        )
        .with_state(app_state)
        .layer(axum::extract::Extension(verifier))
        .layer(axum::extract::Extension(auth_config))
        .layer(axum::extract::Extension(analytics_auth()))
        // The session middleware reads the pool; `AppState` holds built
        // services, not the pool, so it is handed over here as one value.
        .layer(axum::extract::Extension(pg_pool.clone()))
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

/// Why the auth configuration was refused at startup.
#[derive(Debug, thiserror::Error)]
pub enum AuthConfigError {
    #[error(
        "AUTH_SKIP_VERIFY=1 is set, but this binary was built without debug assertions. \
         The bypass exists for local runs and tests; a release build must not authenticate \
         every request as one fabricated user. Unset AUTH_SKIP_VERIFY, or build with debug \
         assertions to use it."
    )]
    BypassInRelease,
    #[error("no PASETO key configured: {0}")]
    NoKey(String),
}

/// Decides whether the dev bypass applies.
///
/// A pure function so the release branch is testable: a debug test binary
/// cannot assert the `!debug_build` arm by running the real thing, and the
/// environment is process-global and races across parallel tests.
pub fn resolve_bypass(
    bypass_requested: bool,
    debug_build: bool,
) -> Result<Option<uuid::Uuid>, AuthConfigError> {
    match (bypass_requested, debug_build) {
        (true, false) => Err(AuthConfigError::BypassInRelease),
        (true, true) => {
            let default_user = std::env::var("AUTH_DEFAULT_USER_ID")
                .ok()
                .and_then(|v| v.parse().ok());
            Ok(Some(
                default_user.unwrap_or_else(auth::verifier::default_user_id),
            ))
        }
        _ => Ok(None),
    }
}

/// Builds the token verifier used by the auth middleware.
///
/// `AUTH_SKIP_VERIFY=1` bypasses verification (local runs and tests only) and
/// is refused in a build without debug assertions, so a release cannot
/// authenticate every request as one fabricated user. Without it the PASETO
/// verifier runs against the keys in `PASETO_KEY_<KID>`, and startup fails
/// loudly when none is configured rather than failing every request with 503.
pub fn build_token_verifier() -> Result<Arc<dyn TokenVerifier>, AuthConfigError> {
    let bypass_requested = std::env::var("AUTH_SKIP_VERIFY").is_ok_and(|v| v == "1");
    let bypass = resolve_bypass(bypass_requested, cfg!(debug_assertions))?;

    if let Some(user_id) = bypass {
        tracing::warn!(
            "AUTH_SKIP_VERIFY=1: every request is authenticated as one fixed user. \
             This is a local development mode."
        );
        return Ok(Arc::new(auth::verifier::StaticUserVerifier::new(Some(
            user_id,
        ))));
    }

    let config = config::auth::AuthConfig::from_env();
    let key_provider = Arc::new(
        auth::key_provider::EnvKeyProvider::for_bearer()
            .map_err(|e| AuthConfigError::NoKey(e.to_string()))?,
    );
    Ok(Arc::new(PasetoVerifier::new(key_provider, config)))
}

/// The credential source for sidecar calls and job rows, when one is configured.
///
/// `None` when no service key is set: the caller then has nothing to enqueue
/// with, and says so rather than writing a job that can only fail.
pub fn analytics_auth() -> Option<Arc<dyn application::outbound::analytics::ServiceTokenSource>> {
    auth::service_token::service_token_source(config::auth::AuthConfig::from_env())
}

/// The analytics backend, chosen once from the environment.
///
/// A sidecar call carries a service token naming the tenant it asks about
/// (D68); without a configured service key the client still builds, and the
/// sidecar refuses its calls loudly instead of the request failing silently.
pub fn analytics_backend() -> Arc<dyn application::outbound::analytics::SpendAnalytics> {
    match std::env::var("ANALYTICS_URL").ok().filter(|u| !u.is_empty()) {
        None => Arc::new(NoAnalytics),
        Some(url) => {
            let auth_config = config::auth::AuthConfig::from_env();
            let client = match auth::service_token::service_token_source(auth_config) {
                Some(source) => application::outbound::http_analytics::HttpAnalyticsClient::new(url)
                    .with_auth(source),
                None => {
                    tracing::warn!(
                        "ANALYTICS_URL is set but no PASETO_SERVICE_KID/PASETO_KEY_* is \
                         configured: sidecar calls will be refused as unauthenticated"
                    );
                    application::outbound::http_analytics::HttpAnalyticsClient::new(url)
                }
            };
            Arc::new(client)
        }
    }
}

/// Handle for the analytics projection, exposed so maintenance commands and
/// tests can drive projection explicitly instead of waiting for the loop.
pub fn analytics_repo(pool: &PgPool) -> PgAnalyticsRepo {
    PgAnalyticsRepo::new(pool.clone())
}

/// Applies the suggestions the worker recorded to the purchases they are about.
///
/// The worker cannot do this itself (D4/D66): it writes `purchase_classification`
/// and returns, and only the application owns `purchase`. Without this step the
/// queue is a dead end -- the worker would record a suggestion that nothing ever
/// applies.
///
/// Returns how many purchases changed.
pub async fn apply_pending_suggestions(
    pool: &PgPool,
    limit: i64,
) -> Result<usize, application::error::ApplicationError> {
    let mut applied = 0;
    let pending = repository::purchase::pending_suggestions(pool, limit)
        .await
        .map_err(|e| application::error::ApplicationError::Internal(e.to_string()))?;
    for (user_id, purchase_id, category_id, confidence, model_version) in pending {
        if repository::purchase::apply_suggested_category(
            pool,
            user_id,
            purchase_id,
            category_id,
            confidence,
            &model_version,
        )
        .await
        .map_err(|e| application::error::ApplicationError::Internal(e.to_string()))?
        {
            applied += 1;
        }
    }
    Ok(applied)
}

/// Runs the analytics projection once, synchronously, to completion.
pub async fn drain_projection<R: application::inbound::repositories::AnalyticsRepository + Send + Sync + 'static>(
    repo: R,
) -> Result<usize, application::error::ApplicationError> {
    application::analytics::projector::FactProjector::new(repo)
        .drain()
        .await
}
