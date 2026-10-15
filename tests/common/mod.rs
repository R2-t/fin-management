//! Shared harness for the database-backed integration tests.
//!
//! Seam: the HTTP router is driven in-process via `tower`'s `oneshot`, against a
//! real TimescaleDB. Assertions may read the database directly for state the
//! HTTP surface cannot express (outbox rows, projection rows, resolution status).
//!
//! The harness deliberately does not initialise telemetry: the global tracing
//! subscriber can only be installed once per process, so tests use
//! `connect_pool` rather than `Config::init`.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use financial_management::auth::verifier::{TokenVerifier, VerifyError};
use financial_management::auth::AuthPrincipal;
use financial_management::config;
use financial_management::repository::analytics::PgAnalyticsRepo;
use financial_management::repository::PgPool;
use financial_management::{build_router, drain_projection};
use serde_json::Value;
use tower::ServiceExt;

/// Connection string for the test database. Fails loudly rather than skipping:
/// a silently skipped integration test is a test that has rotted.
pub fn test_dsn() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             These tests need a TimescaleDB instance with migrations applied.\n\
             Run:  ./scripts/test-db.sh\n\
             then re-run with TEST_DATABASE_URL exported (the script prints it)."
        )
    })
}

/// Connects and asserts the schema is present, so a wrong database produces a
/// clear message instead of a confusing SQL error.
pub async fn test_pool() -> PgPool {
    let pool = financial_management::connect_pool(&test_dsn())
        .await
        .expect("could not connect to TEST_DATABASE_URL");

    {
        let conn = pool.get().await.expect("pool checkout failed");
        let row = conn
            .query_one(
            "SELECT count(*) AS n FROM information_schema.tables
              WHERE table_schema = 'public'
                AND table_name IN ('purchase', 'purchase_fact', 'analytics_outbox', 'wallet')",
            &[],
        )
            .await
            .expect("schema check failed");
        let n: i64 = row.get("n");
        assert_eq!(
            n, 4,
            "test database is missing core tables; run ./scripts/test-db.sh first"
        );
    }

    pool
}

/// A verifier that reads the acting user from the `Authorization` header, so a
/// single test can act as several users (the dev bypass verifier pins one).
pub struct HeaderUserVerifier;

#[async_trait::async_trait]
impl TokenVerifier for HeaderUserVerifier {
    async fn verify(&self, token: &str) -> Result<AuthPrincipal, VerifyError> {
        token
            .trim()
            .parse()
            .map(|user_id| AuthPrincipal { user_id })
            .map_err(|_| VerifyError::InvalidToken)
    }
}

pub fn router(pool: &PgPool) -> axum::Router {
    build_router(pool, Arc::new(HeaderUserVerifier))
}

/// Serialises the mutate-then-project section of a test.
///
/// The projector claims work from the *global* outbox, so if two tests ran this
/// section concurrently, one test's drain could claim another test's row and
/// assert before that row had been written. Each test that records data and then
/// projects must hold this lock.
pub async fn projection_lock() -> tokio::sync::MutexGuard<'static, ()> {
    static PROJECTION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    PROJECTION.lock().await
}

/// A fresh user id per test, so tests are isolated from each other and from any
/// data already in the database.
pub fn new_user() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

fn svc() -> String {
    config::get_service_name()
}

/// Issues a request as `user`, returning the status and parsed JSON body.
pub async fn call(
    app: &axum::Router,
    user: uuid::Uuid,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(format!("/{}{}", svc(), path))
        .header(header::AUTHORIZATION, user.to_string());
    let body = match body {
        Some(v) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };

    let response = app
        .clone()
        .oneshot(builder.body(body).expect("valid request"))
        .await
        .expect("router is infallible");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("reading body");
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

/// Convenience wrappers.
pub async fn get(app: &axum::Router, user: uuid::Uuid, path: &str) -> (StatusCode, Value) {
    call(app, user, "GET", path, None).await
}

pub async fn post(app: &axum::Router, user: uuid::Uuid, path: &str, body: Value) -> (StatusCode, Value) {
    call(app, user, "POST", path, Some(body)).await
}

pub async fn put(app: &axum::Router, user: uuid::Uuid, path: &str, body: Value) -> (StatusCode, Value) {
    call(app, user, "PUT", path, Some(body)).await
}

pub async fn delete(app: &axum::Router, user: uuid::Uuid, path: &str, body: Value) -> (StatusCode, Value) {
    call(app, user, "DELETE", path, Some(body)).await
}

/// Creates a Wallet for `user` and returns its id.
pub async fn create_wallet(app: &axum::Router, user: uuid::Uuid, currency: &str) -> uuid::Uuid {
    let (status, body) = post(
        app,
        user,
        "/wallet",
        serde_json::json!({"name": "test wallet", "currency": currency}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "wallet create failed: {body}");
    body["id"].as_str().expect("wallet id").parse().expect("uuid")
}

/// Records a Purchase of one item and returns its id.
pub async fn create_purchase(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet_id: uuid::Uuid,
    currency: &str,
    unit_price: i64,
) -> uuid::Uuid {
    let (status, body) = post(
        app,
        user,
        "/purchase/purchase_with_items",
        serde_json::json!({
            "wallet_id": wallet_id,
            "store": "Test Store",
            "currency": currency,
            "items": [{"name": "item", "unit_price": unit_price, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "purchase create failed: {body}");
    body["id"].as_str().expect("purchase id").parse().expect("uuid")
}

pub fn facts(pool: &PgPool) -> PgAnalyticsRepo {
    PgAnalyticsRepo::new(pool.clone())
}

/// Runs projection synchronously, instead of waiting for the background loop.
pub async fn project(pool: &PgPool) {
    drain_projection(&facts(pool))
        .await
        .expect("projection drain failed");
}

/// Reads the projection row for a Purchase, if any.
pub async fn fact_row(pool: &PgPool, purchase_id: uuid::Uuid) -> Option<(i64, Option<i64>, Option<i64>, String)> {
    let conn = pool.get().await.expect("pool checkout");
    let row = conn
        .query_opt(
            "SELECT amount_minor, exchange_rate_scaled, amount_base_minor, resolution
               FROM purchase_fact WHERE purchase_id = $1",
            &[&purchase_id],
        )
        .await
        .expect("fact query failed");
    row.map(|r| {
        (
            r.get("amount_minor"),
            r.get("exchange_rate_scaled"),
            r.get("amount_base_minor"),
            r.get("resolution"),
        )
    })
}

/// Pending outbox rows belonging to one user. Scoped so a test's assertion is
/// unaffected by rows other tests leave behind.
pub async fn pending_outbox_for_user(pool: &PgPool, user: uuid::Uuid) -> i64 {
    let conn = pool.get().await.expect("pool checkout");
    let row = conn
        .query_one(
            "SELECT count(*) AS n FROM analytics_outbox
              WHERE processed_at IS NULL AND user_id = $1",
            &[&user],
        )
        .await
        .expect("pending count failed");
    row.get("n")
}

pub async fn count_fact_rows(pool: &PgPool, purchase_id: uuid::Uuid) -> i64 {
    let conn = pool.get().await.expect("pool checkout");
    let row = conn
        .query_one(
            "SELECT count(*) AS n FROM purchase_fact WHERE purchase_id = $1",
            &[&purchase_id],
        )
        .await
        .expect("fact count failed");
    row.get("n")
}

pub async fn execute(pool: &PgPool, sql: &str) {
    let conn = pool.get().await.expect("pool checkout");
    conn.execute(sql, &[]).await.expect("statement failed");
}

/// Reconcile mismatches for one user only, so tests do not interfere with each
/// other's data (a deliberately corrupted row elsewhere would otherwise leak in).
pub async fn mismatches_for(pool: &PgPool, user: uuid::Uuid) -> usize {
    facts(pool)
        .reconcile()
        .await
        .expect("reconcile failed")
        .into_iter()
        .filter(|m| m.user_id == user)
        .count()
}
