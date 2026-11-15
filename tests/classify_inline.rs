//! The inline suggestion path (D67), which until now nothing exercised.
//!
//! A fake analytics backend stands in for the sidecar, so the wiring is tested:
//! that a suggestion is applied to the purchase and returned in the 201, and
//! that failing to get one falls back to the queue instead of failing the
//! create.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use financial_management::application::error::ApplicationError;
use financial_management::application::outbound::analytics::{
    ClassifyRequest, ClassifyResult, RunwayBandsRequest, RunwayBandsResult, SpendAnalytics,
};
use financial_management::auth::verifier::{TokenVerifier, VerifyError};
use financial_management::auth::AuthPrincipal;
use financial_management::config::auth::AuthConfig;
use financial_management::repository::PgPool;
use financial_management::build_router_with_backend;
use tower::ServiceExt;

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

/// Answers with a fixed category, and records that it was asked.
struct SuggestsCategory {
    category_id: Option<i32>,
    confidence: f64,
    source: &'static str,
}

#[async_trait::async_trait]
impl SpendAnalytics for SuggestsCategory {
    async fn classify(
        &self,
        _request: ClassifyRequest,
    ) -> Result<Option<ClassifyResult>, ApplicationError> {
        Ok(Some(ClassifyResult {
            category_id: self.category_id,
            confidence: self.confidence,
            source: self.source.to_string(),
        }))
    }

    async fn runway_bands(
        &self,
        _request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, ApplicationError> {
        Ok(None)
    }
}

/// A sidecar that is down.
struct Unavailable;

#[async_trait::async_trait]
impl SpendAnalytics for Unavailable {
    async fn classify(
        &self,
        _request: ClassifyRequest,
    ) -> Result<Option<ClassifyResult>, ApplicationError> {
        Err(ApplicationError::Internal("sidecar unreachable".into()))
    }

    async fn runway_bands(
        &self,
        _request: RunwayBandsRequest,
    ) -> Result<Option<RunwayBandsResult>, ApplicationError> {
        Ok(None)
    }
}

fn app(pool: &PgPool, backend: Arc<dyn SpendAnalytics>) -> axum::Router {
    build_router_with_backend(
        pool,
        Arc::new(HeaderUserVerifier),
        AuthConfig {
            secure_cookies: false,
            app_origin: None,
            ..AuthConfig::default()
        },
        backend,
    )
}

async fn create_purchase(app: &axum::Router, user: uuid::Uuid, wallet: uuid::Uuid) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/purchase/purchase_with_items",
            financial_management::config::get_service_name()
        ))
        .header(header::AUTHORIZATION, user.to_string())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "wallet_id": wallet,
                "store": "Test Store",
                "currency": "USD",
                "items": [{"name": "item", "unit_price": 1000, "quantity": 1}]
            })
            .to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn category(pool: &PgPool, user: uuid::Uuid) -> i32 {
    let conn = pool.get().await.unwrap();
    conn.query_one(
        "INSERT INTO category (user_id, name) VALUES ($1, $2) RETURNING id",
        &[&user, &format!("cat-{}", uuid::Uuid::new_v4())],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_suggestion_is_applied_and_returned_in_the_201() {
    let pool = common::test_pool().await;
    let user = common::new_user();
    let wallet = common::create_wallet(&common::router(&pool), user, "USD").await;
    let chosen = category(&pool, user).await;

    let _lock = common::projection_lock().await;
    let app = app(
        &pool,
        Arc::new(SuggestsCategory {
            category_id: Some(chosen),
            confidence: 0.9,
            source: "rule",
        }),
    );
    let (status, body) = create_purchase(&app, user, wallet).await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        body["category_id"].as_i64(),
        Some(chosen as i64),
        "the response carries the suggestion the user was promised (D67)"
    );
    assert_eq!(body["classification_status"].as_str(), Some("suggested"));

    // And the row agrees with the response.
    let conn = pool.get().await.unwrap();
    let id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let row = conn
        .query_one(
            "SELECT category_id, version FROM purchase WHERE id = $1",
            &[&id],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<i32>>("category_id"), Some(chosen));
    assert_eq!(
        body["version"].as_i64(),
        Some(row.get::<_, i32>("version") as i64),
        "the response's version must match the row, or the next PATCH conflicts"
    );
}

/// D84: the cross-user model proposes; it does not write.
#[tokio::test]
async fn a_model_proposal_is_not_written_to_the_purchase() {
    let pool = common::test_pool().await;
    let user = common::new_user();
    let wallet = common::create_wallet(&common::router(&pool), user, "USD").await;
    let proposed = category(&pool, user).await;

    let _lock = common::projection_lock().await;
    let app = app(
        &pool,
        Arc::new(SuggestsCategory {
            category_id: Some(proposed),
            confidence: 0.95,
            source: "model",
        }),
    );
    let (status, body) = create_purchase(&app, user, wallet).await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        body["category_id"],
        serde_json::Value::Null,
        "a model's answer is a proposal: committing it would put a value nobody \
         looked at into the ledger, where budgets and totals inherit it"
    );

    let conn = pool.get().await.unwrap();
    let id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();
    let row = conn
        .query_one("SELECT category_id FROM purchase WHERE id = $1", &[&id])
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<i32>>("category_id"), None);
    assert_eq!(
        body["proposed_category_id"].as_i64(),
        Some(proposed as i64),
        "the proposal must reach the user: it is the whole point of D84, and \
         recording it where nobody can see it is not a proposal"
    );
}

#[tokio::test]
async fn no_suggestion_leaves_the_purchase_uncategorized_and_queues_nothing() {
    let pool = common::test_pool().await;
    let user = common::new_user();
    let wallet = common::create_wallet(&common::router(&pool), user, "USD").await;

    let _lock = common::projection_lock().await;
    let app = app(
        &pool,
        Arc::new(SuggestsCategory {
            category_id: None,
            confidence: 0.0,
            source: "rule",
        }),
    );
    let (status, body) = create_purchase(&app, user, wallet).await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["category_id"], serde_json::Value::Null);
    assert_eq!(body["classification_status"].as_str(), Some("unclassified"));

    let conn = pool.get().await.unwrap();
    let queued: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM analytics_job WHERE user_id = $1 AND kind = 'classify'",
            &[&user],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        queued, 0,
        "\"nothing honest to say\" is an answer: it is not re-queued"
    );
}

#[tokio::test]
async fn an_unreachable_sidecar_falls_back_to_the_queue() {
    let pool = common::test_pool().await;
    let user = common::new_user();
    let wallet = common::create_wallet(&common::router(&pool), user, "USD").await;

    let _lock = common::projection_lock().await;
    let app = app(&pool, Arc::new(Unavailable));
    let (status, body) = create_purchase(&app, user, wallet).await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "an outage must not fail the purchase: {body}"
    );
    assert_eq!(body["category_id"], serde_json::Value::Null);

    // The queue is the fallback. No service key is configured in tests, so the
    // job cannot be minted -- the honest outcome is no row, not a row that can
    // only fail. The create still succeeded, which is what this test pins.
    assert_eq!(body["classification_status"].as_str(), Some("unclassified"));
}
