//! Integration tests for transfers: a transfer moves money between two of the
//! user's own Wallets and generates its own ledger legs (ADR-0003).
//!
//! The property that matters is that the legs net to zero. A partial write
//! would silently create or destroy money in aggregate figures (ADR-0004), so
//! the test checks the sum of both wallets' balances, not just one of them.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// A transfer moves the money: it leaves one wallet and arrives in the other,
/// and the total across both is unchanged.
#[tokio::test]
async fn a_transfer_moves_money_between_wallets() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let from = create_wallet(&app, user, "usd").await;
    let to = create_wallet(&app, user, "usd").await;

    // Fund the source wallet with 10.00.
    post(
        &app,
        user,
        &format!("/wallet/{from}/ledger"),
        json!({"amount_minor": 10_000}),
    )
    .await;

    let (status, transfer) = post(
        &app,
        user,
        "/transfer",
        json!({
            "from_wallet_id": from,
            "to_wallet_id": to,
            "amount_minor": 2500
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "transfer create failed: {transfer}");
    assert_eq!(transfer["amount_minor"], 2500);
    assert_eq!(transfer["currency"], "USD");

    let (_, from_balance) = get(&app, user, &format!("/wallet/{from}/balance")).await;
    let (_, to_balance) = get(&app, user, &format!("/wallet/{to}/balance")).await;

    assert_eq!(from_balance["balance_minor"], 7_500, "the source is debited");
    assert_eq!(to_balance["balance_minor"], 2_500, "the destination is credited");

    // The total is unchanged: the legs net to zero.
    let total = from_balance["balance_minor"].as_i64().unwrap()
        + to_balance["balance_minor"].as_i64().unwrap();
    assert_eq!(total, 10_000, "a transfer must not create or destroy money");
}

/// The transfer is a first-class object: it generates both legs, and deleting
/// it reverses them.
#[tokio::test]
async fn deleting_a_transfer_reverses_both_legs() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let from = create_wallet(&app, user, "usd").await;
    let to = create_wallet(&app, user, "usd").await;
    post(
        &app,
        user,
        &format!("/wallet/{from}/ledger"),
        json!({"amount_minor": 10_000}),
    )
    .await;

    let (_, transfer) = post(
        &app,
        user,
        "/transfer",
        json!({
            "from_wallet_id": from,
            "to_wallet_id": to,
            "amount_minor": 2500
        }),
    )
    .await;
    let id = transfer["id"].as_str().expect("transfer id");

    let (status, _) = delete(&app, user, &format!("/transfer/{id}"), json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete failed");

    let (_, from_balance) = get(&app, user, &format!("/wallet/{from}/balance")).await;
    let (_, to_balance) = get(&app, user, &format!("/wallet/{to}/balance")).await;

    assert_eq!(from_balance["balance_minor"], 10_000, "the debit is reversed");
    assert_eq!(to_balance["balance_minor"], 0, "the credit is reversed");
}

/// A transfer to the same wallet is rejected: it would be a no-op that
/// generated two legs.
#[tokio::test]
async fn a_transfer_to_the_same_wallet_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    let (status, _) = post(
        &app,
        user,
        "/transfer",
        json!({
            "from_wallet_id": wallet,
            "to_wallet_id": wallet,
            "amount_minor": 2500
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A transfer between wallets holding different currencies is rejected: a
/// transfer moves money within one currency, it is not a currency exchange.
#[tokio::test]
async fn a_transfer_between_currencies_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let usd = create_wallet(&app, user, "usd").await;
    let eur = create_wallet(&app, user, "eur").await;

    let (status, body) = post(
        &app,
        user,
        "/transfer",
        json!({
            "from_wallet_id": usd,
            "to_wallet_id": eur,
            "amount_minor": 2500
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "must not mix currencies");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("USD"),
        "the error must name the currencies: {body}"
    );
}

/// A zero or negative transfer is rejected: it would generate a credit
/// without a matching debit, or a debit without a credit.
#[tokio::test]
async fn a_non_positive_transfer_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let from = create_wallet(&app, user, "usd").await;
    let to = create_wallet(&app, user, "usd").await;

    for amount in [0, -100] {
        let (status, _) = post(
            &app,
            user,
            "/transfer",
            json!({
                "from_wallet_id": from,
                "to_wallet_id": to,
                "amount_minor": amount
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "amount {amount}");
    }
}

/// One user's transfer is not another's: the source wallet must belong to the
/// caller.
#[tokio::test]
async fn a_transfer_from_another_users_wallet_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let owner_wallet = create_wallet(&app, owner, "usd").await;
    let intruder_wallet = create_wallet(&app, intruder, "usd").await;

    let (status, _) = post(
        &app,
        intruder,
        "/transfer",
        json!({
            "from_wallet_id": owner_wallet,
            "to_wallet_id": intruder_wallet,
            "amount_minor": 2500
        }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user's wallet must not be transferable"
    );
}
