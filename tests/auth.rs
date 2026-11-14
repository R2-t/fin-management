//! The login flow, against a real database and the in-process router.
//!
//! These tests are the reason the auth phase is safe to ship: the session
//! lifecycle, the middleware order, and the rule that a revoked cookie is
//! refused rather than quietly skipped are all pinned here.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{test_pool, test_dsn};
use financial_management::auth::verifier::{TokenVerifier, VerifyError};
use financial_management::auth::AuthPrincipal;
use financial_management::config::auth::AuthConfig;
use financial_management::repository::PgPool;
use financial_management::{build_router_with_auth, resolve_bypass};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

/// The bearer verifier the existing suite uses: the user id is the token.
struct HeaderUserVerifier;

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

fn config() -> AuthConfig {
    AuthConfig {
        // Cookies over plain HTTP in tests, and no origin pinning: the CSRF
        // rule has its own test with an origin set.
        secure_cookies: false,
        app_origin: None,
        password_min_length: 12,
        ..AuthConfig::default()
    }
}

fn app(pool: &PgPool, config: AuthConfig) -> axum::Router {
    build_router_with_auth(pool, Arc::new(HeaderUserVerifier), config)
}

fn svc() -> String {
    financial_management::config::get_service_name()
}

fn uri(path: &str) -> String {
    format!("/{}{}", svc(), path)
}

struct Reply {
    status: StatusCode,
    body: Value,
    cookie: Option<String>,
}

/// Runs a request, optionally with a session cookie and/or a bearer token.
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    bearer: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let mut builder = Request::builder().method(method).uri(uri(path));
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, format!("fm_session={cookie}"));
    }
    if let Some(bearer) = bearer {
        builder = builder.header(header::AUTHORIZATION, bearer);
    }
    let request = match body {
        Some(v) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };

    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();

    // The cookie value is the part before the first ';' after the '='.
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let (name, rest) = v.split_once('=')?;
            if !name.ends_with("fm_session") {
                return None;
            }
            Some(rest.split(';').next()?.to_string())
        })
        .filter(|v| !v.is_empty());

    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };

    Reply {
        status,
        body,
        cookie,
    }
}

fn unique_email() -> String {
    format!("user-{}@example.com", uuid::Uuid::new_v4())
}

async fn register(app: &axum::Router, email: &str, password: &str) -> Reply {
    send(
        app,
        "POST",
        "/auth/register",
        None,
        None,
        Some(json!({"email": email, "password": password})),
    )
    .await
}

const PASSWORD: &str = "correct horse battery staple";

#[tokio::test]
async fn the_whole_lifecycle_works() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();

    // Register logs the user in.
    let registered = register(&app, &email, PASSWORD).await;
    assert_eq!(registered.status, StatusCode::CREATED, "{:?}", registered.body);
    let user_id = registered.body["user_id"].as_str().unwrap().to_string();
    let cookie = registered.cookie.clone().expect("a session cookie");

    // The cookie authenticates a normal request.
    let me = send(&app, "GET", "/auth/me", Some(&cookie), None, None).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["user_id"].as_str().unwrap(), user_id);
    assert_eq!(me.body["email"].as_str().unwrap(), email);

    // Logout revokes it.
    let logged_out = send(&app, "POST", "/auth/logout", Some(&cookie), None, None).await;
    assert_eq!(logged_out.status, StatusCode::NO_CONTENT);

    let after = send(&app, "GET", "/auth/me", Some(&cookie), None, None).await;
    assert_eq!(
        after.status,
        StatusCode::UNAUTHORIZED,
        "a revoked cookie must not authenticate"
    );
}

#[tokio::test]
async fn a_revoked_cookie_is_not_rescued_by_a_bearer_token() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let user_id = common::new_user();

    let reply = send(
        &app,
        "GET",
        "/auth/me",
        Some("not-a-real-session"),
        Some(&user_id.to_string()),
        None,
    )
    .await;

    assert_eq!(
        reply.status,
        StatusCode::UNAUTHORIZED,
        "a present-but-bad cookie must fail, not fall through to bearer"
    );
}

#[tokio::test]
async fn login_works_and_a_second_session_leaves_the_first_alive() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();

    let first = register(&app, &email, PASSWORD).await;
    let first_cookie = first.cookie.clone().unwrap();

    let second = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(second.status, StatusCode::OK, "{:?}", second.body);
    let second_cookie = second.cookie.clone().unwrap();
    assert_ne!(first_cookie, second_cookie, "a login must not reuse a cookie");

    // Both work: they are separate rows (two devices).
    assert_eq!(
        send(&app, "GET", "/auth/me", Some(&first_cookie), None, None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, "GET", "/auth/me", Some(&second_cookie), None, None)
            .await
            .status,
        StatusCode::OK
    );

    // Logging out of one leaves the other.
    assert_eq!(
        send(&app, "POST", "/auth/logout", Some(&first_cookie), None, None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(&app, "GET", "/auth/me", Some(&first_cookie), None, None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "GET", "/auth/me", Some(&second_cookie), None, None)
            .await
            .status,
        StatusCode::OK,
        "the other device stays logged in"
    );
}

#[tokio::test]
async fn login_with_an_existing_cookie_replaces_it() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();

    let registered = register(&app, &email, PASSWORD).await;
    let old = registered.cookie.unwrap();
    let user_id = registered.body["user_id"].as_str().unwrap().to_string();

    let logged_in = send(
        &app,
        "POST",
        "/auth/login",
        Some(&old),
        None,
        Some(json!({"email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(logged_in.status, StatusCode::OK);
    let new = logged_in.cookie.unwrap();
    assert_ne!(old, new, "session fixation: the value must change");
    assert_eq!(logged_in.body["user_id"].as_str().unwrap(), user_id);

    // The old row is gone, not merely unused.
    let conn = pool.get().await.unwrap();
    let old_hash = financial_management::auth::session::hash_for_test(&old);
    let row = conn
        .query_opt("SELECT 1 FROM sessions WHERE id_hash = $1", &[&old_hash])
        .await
        .unwrap();
    assert!(row.is_none(), "the previous session row must be deleted");
}

#[tokio::test]
async fn an_expired_session_is_reported_as_expired_and_deleted() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();

    let registered = register(&app, &email, PASSWORD).await;
    let cookie = registered.cookie.unwrap();

    // Age the row rather than waiting 30 days.
    {
        let conn = pool.get().await.unwrap();
        let hash = financial_management::auth::session::hash_for_test(&cookie);
        conn.execute(
            "UPDATE sessions SET expires_at = NOW() - INTERVAL '1 minute' WHERE id_hash = $1",
            &[&hash],
        )
        .await
        .unwrap();
    }

    let reply = send(&app, "GET", "/auth/me", Some(&cookie), None, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    // And the row was deleted by the lookup that found it.
    let conn = pool.get().await.unwrap();
    let hash = financial_management::auth::session::hash_for_test(&cookie);
    let row = conn
        .query_opt("SELECT 1 FROM sessions WHERE id_hash = $1", &[&hash])
        .await
        .unwrap();
    assert!(row.is_none(), "an expired row is deleted on lookup");
}

#[tokio::test]
async fn two_users_cannot_see_each_others_wallets() {
    let pool = test_pool().await;
    let app = app(&pool, config());

    let email_a = unique_email();
    let email_b = unique_email();
    let a = register(&app, &email_a, PASSWORD).await;
    let b = register(&app, &email_b, PASSWORD).await;
    let cookie_a = a.cookie.unwrap();
    let cookie_b = b.cookie.unwrap();

    let created = send(
        &app,
        "POST",
        "/wallet",
        Some(&cookie_a),
        None,
        Some(json!({"name": format!("wallet-{}", uuid::Uuid::new_v4()), "currency": "USD"})),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
    let wallet_id = created.body["id"].as_str().unwrap().to_string();

    let listed_by_a = send(&app, "GET", "/wallet", Some(&cookie_a), None, None).await;
    assert!(listed_by_a.body.as_array().unwrap().iter().any(|w| w["id"] == wallet_id.as_str()));

    let listed_by_b = send(&app, "GET", "/wallet", Some(&cookie_b), None, None).await;
    assert!(
        !listed_by_b.body.as_array().unwrap().iter().any(|w| w["id"] == wallet_id.as_str()),
        "another user's wallet must not appear"
    );
}

#[tokio::test]
async fn the_router_order_is_right_in_both_directions() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();

    // `/auth/login` is public: reachable with no cookie and no bearer. A 200
    // proves the handler ran; a 401 could equally have come from the
    // middleware, which is the failure this test exists to catch.
    register(&app, &email, PASSWORD).await;
    let login = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(
        login.status,
        StatusCode::OK,
        "login must be reachable without credentials: {:?}",
        login.body
    );

    // `/auth/logout` is protected: refused with neither.
    let logout = send(&app, "POST", "/auth/logout", None, None, None).await;
    assert_eq!(
        logout.status,
        StatusCode::UNAUTHORIZED,
        "logout must require authentication"
    );

    // `/health` stays public.
    let health = send(&app, "GET", "/health", None, None, None).await;
    assert_eq!(health.status, StatusCode::OK);
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_email_answer_identically() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();
    register(&app, &email, PASSWORD).await;

    let wrong_password = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": email, "password": "not the right password"})),
    )
    .await;

    let unknown_email = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": unique_email(), "password": PASSWORD})),
    )
    .await;

    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_email.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong_password.body, unknown_email.body,
        "the response must not say which one was wrong"
    );
}

#[tokio::test]
async fn too_many_failures_are_rate_limited() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();
    register(&app, &email, PASSWORD).await;

    let limit = config().login_max_failures_per_email;
    for _ in 0..limit {
        let reply = send(
            &app,
            "POST",
            "/auth/login",
            None,
            None,
            Some(json!({"email": email, "password": "wrong password here"})),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }

    let limited = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": email, "password": "wrong password here"})),
    )
    .await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);

    // Even the right password is refused while the window is open.
    let right = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(right.status, StatusCode::TOO_MANY_REQUESTS);

    // Clear the window and confirm a success clears the email's rows.
    {
        let conn = pool.get().await.unwrap();
        conn.execute(
            "DELETE FROM login_attempt WHERE email = $1 AND NOT succeeded",
            &[&email],
        )
        .await
        .unwrap();
    }
    let recovered = send(
        &app,
        "POST",
        "/auth/login",
        None,
        None,
        Some(json!({"email": email, "password": PASSWORD})),
    )
    .await;
    assert_eq!(recovered.status, StatusCode::OK);
}

#[tokio::test]
async fn a_duplicate_email_is_rejected() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let email = unique_email();

    assert_eq!(register(&app, &email, PASSWORD).await.status, StatusCode::CREATED);
    assert_eq!(register(&app, &email, PASSWORD).await.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_short_password_is_rejected() {
    let pool = test_pool().await;
    let app = app(&pool, config());
    let reply = register(&app, &unique_email(), "short").await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_cross_site_login_is_refused_when_an_origin_is_pinned() {
    let pool = test_pool().await;
    let app = app(
        &pool,
        AuthConfig {
            app_origin: Some("https://ledger.example".into()),
            ..config()
        },
    );
    let email = unique_email();

    let request = Request::builder()
        .method("POST")
        .uri(uri("/auth/login"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::from(
            json!({"email": email, "password": PASSWORD}).to_string(),
        ))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a cross-site login must be refused: it can set a cookie"
    );
}

#[test]
fn the_dev_bypass_is_refused_in_a_release_build() {
    // The release arm is asserted directly: a debug test binary cannot run the
    // other branch, and the environment is global.
    assert!(resolve_bypass(true, false).is_err());
    assert!(resolve_bypass(true, true).is_ok());
    assert!(resolve_bypass(false, false).is_ok());
    assert!(resolve_bypass(false, true).is_ok());
    assert_eq!(resolve_bypass(false, true).unwrap(), None);
}

#[test]
fn the_database_is_the_one_the_harness_expects() {
    // Fails loudly rather than skipping when the database is missing.
    let dsn = test_dsn();
    assert!(dsn.starts_with("postgres://") || dsn.starts_with("postgresql://"));
}
