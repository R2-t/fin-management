//! Integration tests for budgets: a template plus materialised periods.
//!
//! The properties that matter: a period is materialised lazily with the
//! effective limit the carry rules give it; an overspend lowers the next
//! period's limit; unspent money does NOT raise it; and an edit applies to
//! that period only.

mod common;

use axum::http::StatusCode;
use chrono::Datelike;
use common::*;
use serde_json::json;

/// Records a purchase of `unit_price` dated `days_ago` days before today.
async fn purchase_on(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet: uuid::Uuid,
    unit_price: i64,
    days_ago: i64,
) -> uuid::Uuid {
    let date = chrono::Utc::now() - chrono::Duration::days(days_ago);
    let (status, body) = post(
        app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet,
            "store": "Test Store",
            "currency": "USD",
            "purchase_date": date.to_rfc3339(),
            "items": [{"name": "item", "unit_price": unit_price, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "purchase failed: {body}");
    body["id"].as_str().expect("purchase id").parse().expect("uuid")
}

/// A monthly budget of `amount_minor` starting this month.
async fn monthly_budget(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet: uuid::Uuid,
    amount_minor: i64,
    carry_overspend: bool,
) -> uuid::Uuid {
    let today = chrono::Utc::now().date_naive();
    let first_of_month = today.with_day(1).unwrap();
    let (status, body) = post(
        app,
        user,
        "/budget",
        json!({
            "wallet_id": wallet,
            "name": "Groceries",
            "amount_minor": amount_minor,
            "currency": "USD",
            "cadence": "monthly",
            "reset_day": 1,
            "starts_on": first_of_month.to_string(),
            "carry_overspend": carry_overspend
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "budget create failed: {body}");
    body["id"].as_str().expect("budget id").parse().expect("uuid")
}

/// A budget is a declaration: creating it moves no money.
#[tokio::test]
async fn declaring_a_budget_moves_no_money() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    post(
        &app,
        user,
        &format!("/wallet/{wallet}/ledger"),
        json!({"amount_minor": 100_000}),
    )
    .await;

    monthly_budget(&app, user, wallet, 50_000, true).await;

    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 100_000,
        "a budget is a limit, not a movement"
    );
}

/// The current period reports the spend against it.
#[tokio::test]
async fn the_current_period_reports_the_spend() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    purchase_on(&app, user, wallet, 3000, 0).await;
    purchase_on(&app, user, wallet, 2000, 1).await;
    project(&pool).await;

    let budget = monthly_budget(&app, user, wallet, 50_000, true).await;

    let (status, body) = get(&app, user, &format!("/budget/{budget}/status")).await;
    assert_eq!(status, StatusCode::OK, "status failed: {body}");

    assert_eq!(body["spent_minor"], 5000, "both purchases count: {body}");
    assert_eq!(body["remaining_minor"], 45_000);
    assert_eq!(body["over_budget"], false);
    assert_eq!(body["currency"], "USD");
    assert_eq!(
        body["period"]["amount_minor"], 50_000,
        "the first period's limit is the template's"
    );
}

/// Unspent money is reported, not added to the next period's limit
/// (ADR-0010). The period's unspent figure is visible beside the limit.
#[tokio::test]
async fn unspent_money_is_reported_beside_the_limit() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    purchase_on(&app, user, wallet, 3000, 0).await;
    project(&pool).await;

    let budget = monthly_budget(&app, user, wallet, 50_000, true).await;

    let (_, body) = get(&app, user, &format!("/budget/{budget}/status")).await;

    assert_eq!(body["period"]["amount_minor"], 50_000, "the limit is unchanged");
    assert_eq!(
        body["period"]["unspent_minor"], 47_000,
        "the money left over is reported: {body}"
    );
}

/// An overspend lowers the next period's limit, when the carry flag is on.
#[tokio::test]
async fn an_overspend_lowers_the_next_periods_limit() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    // Spend 70000 against a 50000 limit, in THIS month, so the next period
    // inherits the overspend.
    let today = chrono::Utc::now().date_naive();
    let first_of_month = today.with_day(1).unwrap();
    // The overspend must be dated in the previous period for the carry to
    // apply, so it goes in the month before.
    let previous_month = if first_of_month.month() == 1 {
        chrono::NaiveDate::from_ymd_opt(today.year() - 1, 12, 15).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(today.year(), first_of_month.month() - 1, 15).unwrap()
    };

    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet,
            "store": "Big Shop",
            "currency": "USD",
            "purchase_date": previous_month.and_hms_opt(12, 0, 0).unwrap().and_utc().to_rfc3339(),
            "items": [{"name": "item", "unit_price": 70_000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "purchase failed: {body}");
    project(&pool).await;

    // The budget starts in the previous month, so this month inherits the
    // overspend from it.
    let (status, body) = post(
        &app,
        user,
        "/budget",
        json!({
            "wallet_id": wallet,
            "name": "Groceries",
            "amount_minor": 50_000,
            "currency": "USD",
            "cadence": "monthly",
            "reset_day": 1,
            "starts_on": previous_month.to_string(),
            "carry_overspend": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "budget create failed: {body}");
    let budget: uuid::Uuid = body["id"].as_str().expect("budget id").parse().expect("uuid");

    let (status, body) = get(&app, user, &format!("/budget/{budget}/status")).await;
    assert_eq!(status, StatusCode::OK, "status failed: {body}");

    assert_eq!(
        body["period"]["amount_minor"], 30_000,
        "the overspend of 20000 lowers the next period's limit from 50000 to 30000: {body}"
    );
    assert_eq!(
        body["period"]["template_minor"], 50_000,
        "the template's own limit is kept, so the carry can be explained"
    );
}

/// Without the carry flag, an overspend is forgiven by the reset.
#[tokio::test]
async fn without_the_carry_flag_an_overspend_is_forgiven() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let today = chrono::Utc::now().date_naive();
    let first_of_month = today.with_day(1).unwrap();
    let previous_month = if first_of_month.month() == 1 {
        chrono::NaiveDate::from_ymd_opt(today.year() - 1, 12, 15).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(today.year(), first_of_month.month() - 1, 15).unwrap()
    };

    post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet,
            "store": "Big Shop",
            "currency": "USD",
            "purchase_date": previous_month.and_hms_opt(12, 0, 0).unwrap().and_utc().to_rfc3339(),
            "items": [{"name": "item", "unit_price": 70_000, "quantity": 1}]
        }),
    )
    .await;
    project(&pool).await;

    let (status, body) = post(
        &app,
        user,
        "/budget",
        json!({
            "wallet_id": wallet,
            "name": "Groceries",
            "amount_minor": 50_000,
            "currency": "USD",
            "cadence": "monthly",
            "reset_day": 1,
            "starts_on": previous_month.to_string(),
            "carry_overspend": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "budget create failed: {body}");
    let budget: uuid::Uuid = body["id"].as_str().expect("budget id").parse().expect("uuid");

    let (_, body) = get(&app, user, &format!("/budget/{budget}/status")).await;

    assert_eq!(
        body["period"]["amount_minor"], 50_000,
        "the reset forgives the overspend: {body}"
    );
}

/// A period's limit can be edited, and the edit applies to that period only
/// (ADR-0011).
#[tokio::test]
async fn editing_a_period_changes_only_that_period() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let budget = monthly_budget(&app, user, wallet, 50_000, true).await;

    let today = chrono::Utc::now().date_naive();
    let first_of_month = today.with_day(1).unwrap();

    let (status, body) = put(
        &app,
        user,
        &format!("/budget/{budget}/period"),
        json!({"period_start": first_of_month.to_string(), "amount_minor": 90_000}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "edit failed: {body}");
    assert_eq!(body["amount_minor"], 90_000);
    assert_eq!(
        body["template_minor"], 50_000,
        "the template's limit is unchanged by the edit"
    );

    // The status reflects the edit.
    let (_, body) = get(&app, user, &format!("/budget/{budget}/status")).await;
    assert_eq!(body["period"]["amount_minor"], 90_000);
}

/// One user's budget is not another's.
#[tokio::test]
async fn another_users_budget_is_not_readable() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;
    let budget = monthly_budget(&app, owner, wallet, 50_000, true).await;

    let (status, body) = get(&app, intruder, "/budget").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 0);

    let (status, _) = get(&app, intruder, &format!("/budget/{budget}/status")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = delete(&app, intruder, &format!("/budget/{budget}"), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
