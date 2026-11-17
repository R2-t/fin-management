//! Findings about wrong-looking records: list, dismiss, undo.
//!
//! Detectors are 4.2–4.4 and are not exercised here; these tests raise findings
//! through the repository, then drive the HTTP surface the user sees.

mod common;

use axum::http::StatusCode;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use financial_management::application::inbound::repositories::{
    AnomalyRepository, CreateAnomalyParams,
};
use financial_management::model::anomaly::{Anomaly, AnomalySeverity};
use financial_management::repository::anomaly::PgAnomalyRepo;
use financial_management::repository::PgPool;
use serde_json::json;

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, day, 12, 0, 0).unwrap()
}

async fn raise_finding(
    pool: &PgPool,
    user: uuid::Uuid,
    kind: &str,
    subject_id: Option<uuid::Uuid>,
    key: &str,
    detected_at: DateTime<Utc>,
) -> Anomaly {
    PgAnomalyRepo::new(pool.clone())
        .create(
            user,
            CreateAnomalyParams {
                kind: kind.into(),
                subject_type: "merchant".into(),
                subject_id,
                title: "Two charges look the same".into(),
                detail: json!({"amount_minor": 1200, "currency": "USD"}),
                severity: AnomalySeverity::Notice,
                detected_at: Some(detected_at),
                key: key.into(),
            },
        )
        .await
        .expect("raising a finding")
}

async fn raise(pool: &PgPool, user: uuid::Uuid, key: &str, detected_at: DateTime<Utc>) -> Anomaly {
    raise_finding(pool, user, "duplicate", None, key, detected_at).await
}

fn ids(body: &serde_json::Value) -> Vec<String> {
    body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_finding_is_listed_dismissed_and_undismissed() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    let finding = raise(&pool, user, "1200|2026-01-01", at(1)).await;
    let id = finding.id.to_string();

    let (status, listed) = common::get(&app, user, "/anomaly").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(ids(&listed).contains(&id), "open findings are the default");

    let (status, _) = common::post(
        &app,
        user,
        &format!("/anomaly/{id}/dismiss"),
        json!({"reason": "I did buy two"}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, open) = common::get(&app, user, "/anomaly").await;
    assert!(!ids(&open).contains(&id), "dismissing closes it");

    let (_, dismissed) = common::get(&app, user, "/anomaly?state=dismissed").await;
    assert!(ids(&dismissed).contains(&id));
    assert_eq!(dismissed["items"][0]["dismissed_reason"], "I did buy two");

    let (status, _) = common::post(&app, user, &format!("/anomaly/{id}/undismiss"), json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, open) = common::get(&app, user, "/anomaly").await;
    assert!(ids(&open).contains(&id), "undismiss restores it");
}

#[tokio::test]
async fn listing_can_filter_by_kind() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    let duplicate = raise(&pool, user, "1200|2026-01-01", at(1)).await;
    let outlier = raise_finding(
        &pool,
        user,
        "amount_outlier",
        None,
        "3000|2026-01-02",
        at(2),
    )
    .await;

    let (status, listed) = common::get(&app, user, "/anomaly?kind=amount_outlier").await;
    assert_eq!(status, StatusCode::OK);
    let listed = ids(&listed);
    assert_eq!(listed, vec![outlier.id.to_string()]);
    assert!(!listed.contains(&duplicate.id.to_string()));
}

#[tokio::test]
async fn a_page_of_exactly_limit_items_returns_its_last_row() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    raise(&pool, user, "1200|2026-01-01", at(1)).await;
    raise(&pool, user, "1300|2026-01-02", at(2)).await;

    let (status, listed) = common::get(&app, user, "/anomaly?limit=2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&listed).len(), 2);
    assert!(listed["next_cursor"].is_null());
}

#[tokio::test]
async fn a_long_dismissal_reason_is_refused() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    let finding = raise(&pool, user, "1200|2026-01-01", at(1)).await;

    let (status, _) = common::post(
        &app,
        user,
        &format!("/anomaly/{}/dismiss", finding.id),
        json!({"reason": "x".repeat(501)}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_same_evidence_raises_one_finding() {
    let pool = common::test_pool().await;
    let user = common::new_user();

    let first = raise(&pool, user, "1200|2026-01-01", at(1)).await;
    let again = raise(&pool, user, "1200|2026-01-01", at(1)).await;
    assert_eq!(first.id, again.id, "re-detecting unchanged evidence is idempotent");

    let changed = raise(&pool, user, "1300|2026-01-01", at(1)).await;
    assert_ne!(first.id, changed.id, "new evidence is a new finding");

    // The same rule must hold for purchase-level findings, which use the
    // non-null side of the unique index.
    let subject = uuid::Uuid::new_v4();
    let purchase_finding =
        raise_finding(&pool, user, "duplicate", Some(subject), "purchase|1200", at(2)).await;
    let purchase_again =
        raise_finding(&pool, user, "duplicate", Some(subject), "purchase|1200", at(2)).await;
    assert_eq!(purchase_finding.id, purchase_again.id);

    // A dismissed finding stays dismissed even when the same evidence is
    // detected again; the user already answered.
    PgAnomalyRepo::new(pool.clone())
        .dismiss(user, first.id, None)
        .await
        .unwrap();
    let after_dismiss = raise(&pool, user, "1200|2026-01-01", at(1)).await;
    assert_eq!(after_dismiss.id, first.id);
    assert!(after_dismiss.dismissed_at.is_some());
}

#[tokio::test]
async fn another_users_findings_are_invisible() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let alice = common::new_user();
    let bob = common::new_user();
    let finding = raise(&pool, alice, "1200|2026-01-01", at(1)).await;
    let id = finding.id.to_string();

    let (_, listed) = common::get(&app, bob, "/anomaly").await;
    assert!(!ids(&listed).contains(&id));

    let (status, _) = common::post(&app, bob, &format!("/anomaly/{id}/dismiss"), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pages_use_a_stable_cursor() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    for day in 1..=3 {
        raise(&pool, user, &format!("{day}00|2026-01-{day:02}"), at(day)).await;
    }

    let (status, first) = common::get(&app, user, "/anomaly?limit=2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&first).len(), 2);
    let cursor = first["next_cursor"].as_str().expect("a full page has a cursor");

    let (status, second) = common::get(&app, user, &format!("/anomaly?limit=2&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&second).len(), 1, "the last row is returned");
    assert!(second["next_cursor"].is_null(), "the last page has no cursor");

    // A cursor older than every finding is past the end.
    let past = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        "{}|{}",
        (at(1) - chrono::Duration::hours(1)).to_rfc3339(),
        uuid::Uuid::new_v4()
    ));
    let (status, empty) = common::get(&app, user, &format!("/anomaly?cursor={past}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ids(&empty).is_empty());
    assert!(empty["next_cursor"].is_null());
}

#[tokio::test]
async fn a_malformed_cursor_is_refused() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();

    let (status, _) = common::get(&app, user, "/anomaly?cursor=not-a-cursor").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
