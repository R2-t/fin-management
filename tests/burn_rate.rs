//! Integration tests for the recurring rules and the burn rate.
//!
//! The property that matters: a subscription declared as a rule is excluded
//! from the spending pace, so a forecast does not subtract it twice -- once
//! inside the pace and once as a dated movement.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// Records a purchase dated `days_ago` days before today.
async fn purchase_on(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet: uuid::Uuid,
    unit_price: i64,
    days_ago: i64,
    store: &str,
) -> uuid::Uuid {
    let date = chrono::Utc::now() - chrono::Duration::days(days_ago);
    let (status, body) = post(
        app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet,
            "store": store,
            "currency": "USD",
            "purchase_date": date.to_rfc3339(),
            "items": [{"name": "item", "unit_price": unit_price, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "purchase failed: {body}");
    body["id"].as_str().expect("purchase id").parse().expect("uuid")
}

/// A rule for a monthly subscription of `amount_minor` starting today.
async fn monthly_rule(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet: uuid::Uuid,
    amount_minor: i64,
    merchant_id: Option<uuid::Uuid>,
) -> uuid::Uuid {
    let today = chrono::Utc::now().date_naive();
    let (status, body) = post(
        app,
        user,
        "/recurring",
        json!({
            "wallet_id": wallet,
            "merchant_id": merchant_id,
            "name": "Subscription",
            "amount_minor": amount_minor,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": today.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "rule create failed: {body}");
    body["id"].as_str().expect("rule id").parse().expect("uuid")
}

/// The merchant a purchase was recorded against.
async fn merchant_of(app: &axum::Router, user: uuid::Uuid, purchase_id: uuid::Uuid) -> Option<uuid::Uuid> {
    let (_, body) = get(app, user, &format!("/purchase/{purchase_id}")).await;
    body["merchant_id"]
        .as_str()
        .map(|s| s.parse().expect("uuid"))
}

/// A rule is a declaration: creating it moves no money.
#[tokio::test]
async fn declaring_a_recurring_rule_moves_no_money() {
    // The projector claims work from the GLOBAL outbox, so an unlocked test
    // can have its rows claimed by another test's drain.
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

    monthly_rule(&app, user, wallet, 1500, None).await;

    let (_, balance) = get(&app, user, &format!("/wallet/{wallet}/balance")).await;
    assert_eq!(
        balance["balance_minor"], 100_000,
        "a rule is a declaration, not a movement"
    );
}

/// A rule cannot be in a currency the wallet does not hold: that would need a
/// rate the user has not given us (ADR-0002).
#[tokio::test]
async fn a_rule_in_another_currency_is_rejected() {
    // The projector claims work from the GLOBAL outbox, so an unlocked test
    // can have its rows claimed by another test's drain.
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let today = chrono::Utc::now().date_naive();

    let (status, body) = post(
        &app,
        user,
        "/recurring",
        json!({
            "wallet_id": wallet,
            "name": "Subscription",
            "amount_minor": 1500,
            "currency": "EUR",
            "cadence": "monthly",
            "starts_on": today.to_string()
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap_or_default().contains("USD"),
        "the error must name the wallet's currency: {body}"
    );
}

/// The pace excludes the recorded payment of a declared rule.
#[tokio::test]
async fn the_burn_rate_excludes_a_declared_subscriptions_payment() {
    // The projector claims work from the GLOBAL outbox, so an unlocked test
    // can have its rows claimed by another test's drain.
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;

    // A subscription of 1500 a month, paid today.
    let subscription = purchase_on(&app, user, wallet, 1500, 0, "Netflix").await;
    let merchant = merchant_of(&app, user, subscription).await;

    // Plus ordinary spending, so there is a pace to measure.
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 1000, days_ago, "Grocery Store").await;
    }
    project(&pool).await;

    let (_, before) = get(&app, user, "/analytics/burn-rate").await;
    assert_eq!(
        before["history_days"], 15,
        "the window is the days that have history, inclusive of today"
    );

    // Without a rule, the subscription is part of the pace.
    let total = before["days"]
        .as_array()
        .expect("array")
        .iter()
        .map(|d| d["total_minor"].as_i64().unwrap())
        .sum::<i64>();
    assert!(total >= 1500, "the subscription is in the total spend");

    // Declare it a rule.
    monthly_rule(&app, user, wallet, 1500, merchant).await;

    let (_, after) = get(&app, user, "/analytics/burn-rate").await;
    let discretionary = after["days"]
        .as_array()
        .expect("array")
        .iter()
        .map(|d| d["discretionary_minor"].as_i64().unwrap())
        .sum::<i64>();
    let total_after = after["days"]
        .as_array()
        .expect("array")
        .iter()
        .map(|d| d["total_minor"].as_i64().unwrap())
        .sum::<i64>();

    assert_eq!(total, total_after, "the total spend does not change");
    assert_eq!(
        discretionary,
        total_after - 1500,
        "the subscription's payment is excluded from the pace"
    );
}

/// With too little history, no pace is reported at all: inventing one is
/// refused (D42). A zero would read as a measured pace of no spending.
#[tokio::test]
async fn too_little_history_reports_no_pace() {
    // The projector claims work from the GLOBAL outbox, so an unlocked test
    // can have its rows claimed by another test's drain.
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    purchase_on(&app, user, wallet, 1000, 0, "Grocery Store").await;
    project(&pool).await;

    let (status, body) = get(&app, user, "/analytics/burn-rate").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body["history_days"].as_i64().unwrap() < 7,
        "a single day is not a pace"
    );
    assert_eq!(
        body["daily_minor"],
        serde_json::Value::Null,
        "below a week of history, no pace is reported: {body}"
    );
    assert_eq!(body["source"], "insufficient_history", "{body}");
}

/// A cancelled rule stops being forecast: its payments are part of the pace
/// again.
#[tokio::test]
async fn a_cancelled_rule_stops_excluding_its_payments() {
    // The projector claims work from the GLOBAL outbox, so an unlocked test
    // can have its rows claimed by another test's drain.
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let subscription = purchase_on(&app, user, wallet, 1500, 0, "Netflix").await;
    let merchant = merchant_of(&app, user, subscription).await;
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 1000, days_ago, "Grocery Store").await;
    }
    project(&pool).await;

    let rule = monthly_rule(&app, user, wallet, 1500, merchant).await;

    let (_, excluded) = get(&app, user, "/analytics/burn-rate").await;
    let excluded_sum = excluded["days"]
        .as_array()
        .expect("array")
        .iter()
        .map(|d| d["discretionary_minor"].as_i64().unwrap())
        .sum::<i64>();

    let (status, _) = post(&app, user, &format!("/recurring/{rule}/cancel"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "cancel failed");

    let (_, included) = get(&app, user, "/analytics/burn-rate").await;
    let included_sum = included["days"]
        .as_array()
        .expect("array")
        .iter()
        .map(|d| d["discretionary_minor"].as_i64().unwrap())
        .sum::<i64>();

    assert_eq!(
        included_sum,
        excluded_sum + 1500,
        "a cancelled rule's payment is discretionary again"
    );
}

/// A pace belongs to one currency. Asking across two is refused rather than
/// reporting one of them as if it spoke for both (ADR-0002).
#[tokio::test]
async fn a_burn_rate_across_two_currencies_is_refused() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let usd = create_wallet(&app, user, "usd").await;
    let eur = create_wallet(&app, user, "eur").await;
    purchase_on(&app, user, usd, 1000, 0, "Grocery Store").await;
    purchase_on(&app, user, eur, 1000, 0, "Grocery Store").await;
    project(&pool).await;

    let (status, body) = get(&app, user, "/analytics/burn-rate").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("currency"),
        "the error must say why: {body}"
    );

    // Naming a wallet gives the pace in that wallet's currency.
    let (status, body) = get(&app, user, &format!("/analytics/burn-rate?wallet_id={usd}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["currency"], "USD", "the pace says what it is in: {body}");
}

/// One user's rules are not another's.
#[tokio::test]
async fn a_rule_from_another_user_is_not_visible() {
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;
    monthly_rule(&app, owner, wallet, 1500, None).await;

    let (status, body) = get(&app, intruder, "/recurring").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_array().expect("array").len(),
        0,
        "another user's rules must not be visible"
    );
}
