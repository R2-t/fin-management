//! Integration tests for the ledger: a Wallet's balance is the sum of its
//! entries (ADR-0004).
//!
//! Seam: the HTTP router against a real TimescaleDB. The ledger is read
//! through the API; the entries themselves are read from the database, because
//! the HTTP surface cannot express the sum the balance is defined as.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// A Wallet with no entries has a balance of zero, not an error.
#[tokio::test]
async fn a_new_wallet_has_a_zero_balance() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    let (status, body) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;

    assert_eq!(status, StatusCode::OK, "balance read failed: {body}");
    assert_eq!(body["balance_minor"], 0);
    assert_eq!(body["currency"], "USD");
    assert_eq!(body["currency_minor_unit"], 2);
    assert_eq!(body["entry_count"], 0);
}

/// The balance is the SUM of the entries: an opening balance plus a later
/// adjustment, in both directions.
#[tokio::test]
async fn the_balance_is_the_sum_of_the_entries() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    // Open the wallet with 10.00.
    let (status, body) = post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 1000}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "opening balance failed: {body}");
    assert_eq!(body["amount_minor"], 1000);
    assert_eq!(body["kind"], "opening_balance");

    // Adjust it down by 2.50.
    let (status, body) = post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": -250}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "adjustment failed: {body}");
    assert_eq!(body["kind"], "adjustment");

    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 750,
        "balance must be the sum of the entries"
    );
    assert_eq!(balance["entry_count"], 2);

    // The entries are readable, newest first.
    let (status, entries) = get(&app, user, &format!("/wallet/{wallet}/ledger")).await;
    assert_eq!(status, StatusCode::OK);
    let entries = entries.as_array().expect("array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["amount_minor"], -250);
    assert_eq!(entries[1]["amount_minor"], 1000);
}

/// A ledger entry in a currency the Wallet does not hold is rejected: it would
/// make the balance a sum across currencies (ADR-0002).
#[tokio::test]
async fn a_ledger_entry_in_another_currency_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    let (status, body) = post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 1000, "currency": "EUR"}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "must not mix currencies");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("USD"),
        "the error must name the wallet's currency: {body}"
    );
}

/// One user's ledger is not another's.
#[tokio::test]
async fn a_wallet_balance_is_not_readable_by_another_user() {
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;
    post(
        &app,
        owner,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 1000}),
    )
    .await;

    let (status, _) = get(&app, intruder, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user must not read the balance"
    );

    let (status, _) = get(&app, intruder, &format!("/wallet/{wallet}/ledger")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user must not read the entries"
    );

    let (status, _) = post(
        &app,
        intruder,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 5000}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user must not write to the ledger"
    );
}

/// A zero adjustment is rejected: it records nothing and only adds noise.
#[tokio::test]
async fn a_zero_adjustment_is_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    let (status, _) = post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 0}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A Purchase debits the wallet in the same transaction that records it
/// (ADR-0007). The debit is the purchase's amount in the wallet's currency.
#[tokio::test]
async fn a_purchase_debits_the_wallet() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 10_000}),
    )
    .await;

    // 25.00 spent.
    create_purchase(&app, user, wallet, "USD", 2500).await;

    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 7_500,
        "the purchase must debit the wallet"
    );

    let (_, entries) = get(&app, user, &format!("/wallet/{wallet}/ledger")).await;
    let entries = entries.as_array().expect("array");
    let debit = entries
        .iter()
        .find(|e| e["kind"] == "purchase")
        .expect("a purchase debit entry");
    assert_eq!(debit["amount_minor"], -2500, "a debit is negative");
}

/// Deleting a Purchase stops the money being spent: the debit is superseded
/// and the balance returns to what it was.
#[tokio::test]
async fn deleting_a_purchase_reverses_the_debit() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 10_000}),
    )
    .await;

    let purchase = create_purchase(&app, user, wallet, "USD", 2500).await;

    let (_, before) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(before["balance_minor"], 7_500);

    let (status, _) = delete(
        &app,
        user,
        &format!("/purchase/{purchase}"),
        json!({"current_version": 1}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete failed");

    let (_, after) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        after["balance_minor"], 10_000,
        "a deleted purchase must not keep spending money"
    );
}

/// A purchase whose rate is unresolved has no converted amount, so it must not
/// debit the wallet with a made-up number (ADR-0006).
#[tokio::test]
async fn an_unresolved_purchase_does_not_debit_the_wallet() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    // A purchase in a currency the wallet does not hold, with no rate
    // available: the write succeeds and the conversion is left unresolved.
    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet,
            "store": "Foreign Store",
            "currency": "EUR",
            "items": [{"name": "item", "unit_price": 5000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "purchase create failed: {body}");

    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 0,
        "an unresolved conversion must not debit the wallet"
    );
}
