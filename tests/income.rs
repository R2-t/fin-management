//! Integration tests for income: a schedule is an expectation, and it becomes
//! money only when the user records that it arrived (ADR-0008).

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// A schedule is an expectation: creating it must NOT credit the wallet.
#[tokio::test]
async fn declaring_income_does_not_move_money() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    let (status, body) = post(
        &app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": "2026-01-01",
            "allocations": [{"wallet_id": wallet, "amount_minor": 200_000}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    assert_eq!(body["amount_minor"], 200_000);
    assert_eq!(body["cadence"], "monthly");

    // The money has not arrived. The ledger is empty.
    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 0,
        "an expectation is not money until it is received"
    );
}

/// Receiving the occurrence credits the wallet.
#[tokio::test]
async fn receiving_income_credits_the_wallet() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    let (_, schedule) = post(
        &app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": "2026-01-01",
            "allocations": [{"wallet_id": wallet, "amount_minor": 200_000}]
        }),
    )
    .await;
    let id = schedule["id"].as_str().expect("schedule id");

    let (status, receipt) = post(&app, user, &format!("/income/{id}/receive"), json!({})).await;
    assert_eq!(status, StatusCode::CREATED, "receive failed: {receipt}");
    assert_eq!(receipt["amount_minor"], 200_000);

    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 200_000,
        "a received occurrence credits the wallet"
    );

    // The credit is a ledger entry, so it shows in the history.
    let (_, entries) = get(&app, user, &format!("/wallet/{wallet}/ledger")).await;
    let entries = entries.as_array().expect("array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "income");
    assert_eq!(entries[0]["amount_minor"], 200_000);
}

/// An income split across two wallets credits both, by the split amounts.
#[tokio::test]
async fn income_split_across_wallets_credits_each() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let rent = create_wallet(&app, user, "usd").await;
    let savings = create_wallet(&app, user, "usd").await;

    let (_, schedule) = post(
        &app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": "2026-01-01",
            "allocations": [
                {"wallet_id": rent, "amount_minor": 150_000},
                {"wallet_id": savings, "amount_minor": 50_000}
            ]
        }),
    )
    .await;
    let id = schedule["id"].as_str().expect("schedule id");

    let (status, _) = post(&app, user, &format!("/income/{id}/receive"), json!({})).await;
    assert_eq!(status, StatusCode::CREATED);

    let (_, rent_balance) = get(&app, user, &format!("/wallet/{rent}/balance")).await;
    let (_, savings_balance) = get(&app, user, &format!("/wallet/{savings}/balance")).await;

    assert_eq!(rent_balance["balance_minor"], 150_000);
    assert_eq!(savings_balance["balance_minor"], 50_000);
}

/// The allocations must account for the whole amount: a schedule that pays out
/// less would create money on receipt, and one that pays out more would
/// destroy it.
#[tokio::test]
async fn allocations_that_do_not_sum_to_the_amount_are_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    // Pays out 100 of a 200 schedule.
    let (status, body) = post(
        &app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": "2026-01-01",
            "allocations": [{"wallet_id": wallet, "amount_minor": 100_000}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or_default().contains("allocations"),
        "the error must say the allocations do not add up: {body}"
    );
}

/// A schedule cannot direct money into a wallet holding another currency:
/// that would need a rate the user has not given us (ADR-0002).
#[tokio::test]
async fn income_into_another_currency_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let usd = create_wallet(&app, user, "usd").await;

    let (status, body) = post(
        &app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "EUR",
            "cadence": "monthly",
            "starts_on": "2026-01-01",
            "allocations": [{"wallet_id": usd, "amount_minor": 200_000}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or_default().contains("USD"),
        "the error must name the wallet's currency: {body}"
    );
}

/// One user's income schedule is not another's.
#[tokio::test]
async fn another_users_schedule_cannot_be_received() {
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;
    let (_, schedule) = post(
        &app,
        owner,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": "2026-01-01",
            "allocations": [{"wallet_id": wallet, "amount_minor": 200_000}]
        }),
    )
    .await;
    let id = schedule["id"].as_str().expect("schedule id");

    let (status, _) = post(&app, intruder, &format!("/income/{id}/receive"), json!({})).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user must not receive income into their own wallet"
    );

    // And the owner's wallet is untouched.
    let (_, balance) = get(&app, owner, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(balance["balance_minor"], 0);
}
