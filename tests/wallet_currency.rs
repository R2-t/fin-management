//! Integration tests for a wallet currency change.
//!
//! Two rules: a wallet that holds entries keeps its currency (D50), and a
//! wallet that does not has its budgets, income schedules and recurring rules
//! disabled, because they are all denominated in the old currency (D32, D51).

mod common;

use axum::http::StatusCode;
use chrono::Datelike;
use common::*;
use serde_json::json;

/// A budget, an income schedule and a recurring rule, all pointed at `wallet`.
async fn declare_things(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
    let today = chrono::Utc::now().date_naive();
    let first_of_month = today.with_day(1).unwrap();

    let (status, body) = post(
        app,
        user,
        "/budget",
        json!({
            "wallet_id": wallet,
            "name": "Groceries",
            "amount_minor": 50_000,
            "currency": "USD",
            "cadence": "monthly",
            "reset_day": 1,
            "starts_on": first_of_month.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "budget: {body}");
    let budget: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let (status, body) = post(
        app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 200_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": today.to_string(),
            "allocations": [{"wallet_id": wallet, "amount_minor": 200_000}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "income: {body}");
    let income: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let (status, body) = post(
        app,
        user,
        "/recurring",
        json!({
            "wallet_id": wallet,
            "name": "Rent",
            "amount_minor": 10_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": today.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "recurring: {body}");
    let recurring: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();

    (budget, income, recurring)
}

/// A wallet that holds entries keeps its currency: converting them would need
/// a rate for a past date that was never recorded (D50).
#[tokio::test]
async fn a_wallet_with_entries_keeps_its_currency() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 1000}),
    )
    .await;

    let (status, body) = put(
        &app,
        user,
        &format!("/wallet/{wallet}"),
        json!({"name": "travel", "currency": "EUR"}),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a wallet holding a balance cannot change currency: {body}"
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("USD"),
        "the error must name the currency it holds: {body}"
    );

    // The balance is untouched and still labelled with the currency it is in.
    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(balance["currency"], "USD");
    assert_eq!(balance["balance_minor"], 1000);
}

/// Changing the currency of an empty wallet disables what was denominated in
/// the old one: a budget limit, an income amount and a recurring charge are
/// all stated in that currency (D32, D51).
#[tokio::test]
async fn changing_an_empty_wallets_currency_disables_what_it_denominated() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let (budget, income, recurring) = declare_things(&app, user, wallet).await;

    let (status, body) = put(
        &app,
        user,
        &format!("/wallet/{wallet}"),
        json!({"name": "travel", "currency": "EUR"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "an empty wallet may change: {body}");
    assert_eq!(body["currency"], "EUR");

    let (_, budgets) = get(&app, user, "/budget").await;
    let budget_row = budgets
        .as_array()
        .expect("array")
        .iter()
        .find(|b| b["id"] == budget.to_string())
        .expect("the budget");
    assert_eq!(
        budget_row["active"], false,
        "a budget limit in USD no longer applies to a EUR wallet"
    );

    let (_, incomes) = get(&app, user, "/income").await;
    let income_row = incomes
        .as_array()
        .expect("array")
        .iter()
        .find(|i| i["id"] == income.to_string())
        .expect("the income schedule");
    assert_eq!(
        income_row["active"], false,
        "an income amount in USD no longer applies to a EUR wallet"
    );

    let (_, recurring_rules) = get(&app, user, "/recurring").await;
    let recurring_row = recurring_rules
        .as_array()
        .expect("array")
        .iter()
        .find(|r| r["id"] == recurring.to_string())
        .expect("the recurring rule");
    assert_eq!(
        recurring_row["status"], "cancelled",
        "a recurring charge in USD no longer applies to a EUR wallet"
    );
}

/// Renaming a wallet must not cascade: the currency did not change, so
/// nothing denominated in it stopped applying.
#[tokio::test]
async fn renaming_a_wallet_disables_nothing() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let (budget, _, _) = declare_things(&app, user, wallet).await;

    let (status, body) = put(
        &app,
        user,
        &format!("/wallet/{wallet}"),
        json!({"name": "renamed", "currency": "USD"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, budgets) = get(&app, user, "/budget").await;
    let budget_row = budgets
        .as_array()
        .expect("array")
        .iter()
        .find(|b| b["id"] == budget.to_string())
        .expect("the budget");
    assert_eq!(
        budget_row["active"], true,
        "setting the same currency again is not a currency change"
    );
}

/// One user's wallet cannot be re-denominated by another.
#[tokio::test]
async fn another_users_wallet_cannot_be_changed() {
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;

    let (status, _) = put(
        &app,
        intruder,
        &format!("/wallet/{wallet}"),
        json!({"name": "hijacked", "currency": "EUR"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, balance) = get(&app, owner, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(balance["currency"], "USD");
}
