//! Account groups: a named set of wallets, for reporting.
//!
//! A group is a filter, not a consolidation — the app never sums across
//! currencies (ADR-0002) — so these tests assert membership and tenancy, and
//! deliberately assert no total.

mod common;

use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn a_group_is_created_listed_and_renamed() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();

    let (status, created) = common::post(&app, user, "/account-group", json!({"name": " Savings "})).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "Savings", "the name is trimmed");
    let id = created["id"].as_str().unwrap().to_string();

    let (status, listed) = common::get(&app, user, "/account-group").await;
    assert_eq!(status, StatusCode::OK);
    assert!(listed.as_array().unwrap().iter().any(|g| g["id"] == id.as_str()));

    let (status, renamed) = common::call(
        &app, user, "PATCH", &format!("/account-group/{id}"), Some(json!({"name": "Long-term"})),
    ).await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Long-term");
}

#[tokio::test]
async fn a_duplicate_name_is_refused() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();

    common::post(&app, user, "/account-group", json!({"name": "Savings"})).await;
    let (status, _) = common::post(&app, user, "/account-group", json!({"name": "savings"})).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a name differing only in case is the same group"
    );
}

#[tokio::test]
async fn a_wallet_can_be_grouped_and_ungrouped() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    let wallet = common::create_wallet(&app, user, "USD").await;

    let (_, group) = common::post(&app, user, "/account-group", json!({"name": "Current"})).await;
    let group_id = group["id"].as_str().unwrap().to_string();

    let (status, _) = common::call(
        &app, user, "PUT", &format!("/account-group/wallet/{wallet}"), Some(json!({"group_id": group_id})),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Ungrouping is the same call with null.
    let (status, _) = common::call(
        &app, user, "PUT", &format!("/account-group/wallet/{wallet}"), Some(json!({"group_id": null})),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn deleting_a_group_ungroups_its_wallets() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let user = common::new_user();
    let wallet = common::create_wallet(&app, user, "USD").await;

    let (_, group) = common::post(&app, user, "/account-group", json!({"name": "Savings"})).await;
    let group_id = group["id"].as_str().unwrap().to_string();
    common::call(
        &app, user, "PUT", &format!("/account-group/wallet/{wallet}"), Some(json!({"group_id": group_id})),
    ).await;

    let (status, _) = common::delete(&app, user, &format!("/account-group/{group_id}"), serde_json::Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The wallet must not be left pointing at a soft-deleted group: that would
    // make it invisible to every group report while looking grouped.
    let conn = pool.get().await.unwrap();
    let row = conn
        .query_one("SELECT account_group_id FROM wallet WHERE id = $1", &[&wallet])
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<uuid::Uuid>>("account_group_id"), None);
}

#[tokio::test]
async fn another_users_group_cannot_be_used() {
    let pool = common::test_pool().await;
    let app = common::router(&pool);
    let alice = common::new_user();
    let bob = common::new_user();
    let bobs_wallet = common::create_wallet(&app, bob, "USD").await;

    let (_, alices_group) = common::post(&app, alice, "/account-group", json!({"name": "Mine"})).await;
    let group_id = alices_group["id"].as_str().unwrap().to_string();

    // Bob cannot put his wallet into Alice's group...
    let (status, _) = common::call(
        &app, bob, "PUT", &format!("/account-group/wallet/{bobs_wallet}"), Some(json!({"group_id": group_id})),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // ...and Alice cannot see his groups, because she has none of his.
    let (_, listed) = common::get(&app, bob, "/account-group").await;
    assert!(
        !listed.as_array().unwrap().iter().any(|g| g["id"] == group_id.as_str()),
        "one user's groups are invisible to another"
    );
}
