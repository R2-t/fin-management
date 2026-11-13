//! Integration tests for the runway projection.
//!
//! The projection walks forward from the ledger balance and reports the
//! trough and the recovery date (D38). These tests pin the cases where the
//! answer is easy to get wrong: a balance that is already negative, one that
//! never recovers, and one that a paycheck pulls back.

mod common;

use axum::http::StatusCode;
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

/// A wallet with `balance_minor` and a month of spending at `daily` a day.
async fn wallet_with_spending(
    app: &axum::Router,
    pool: &financial_management::repository::PgPool,
    user: uuid::Uuid,
    balance_minor: i64,
    daily: i64,
) -> uuid::Uuid {
    let wallet = create_wallet(app, user, "usd").await;
    if balance_minor != 0 {
        post(
            app,
            user,
            &format!("/wallet/{wallet}/ledger"),
            json!({"amount_minor": balance_minor}),
        )
        .await;
    }
    // Two weeks of spending, so there is enough history for a pace.
    for days_ago in 1..=14 {
        purchase_on(app, user, wallet, daily, days_ago).await;
    }
    project(pool).await;
    wallet
}

/// A wallet with a generous balance: the projection should find it solvent
/// across the whole horizon.
#[tokio::test]
async fn a_well_funded_wallet_has_no_trough_and_no_recovery() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = wallet_with_spending(&app, &pool, user, 1_000_000, 1000).await;

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK, "runway failed: {body}");

    // The recorded spending debited the wallet, so the balance the projection
    // starts from is the opening balance less what was spent.
    assert_eq!(body["starting_balance_minor"], 1_000_000 - 14_000);
    assert_eq!(body["horizon_days"], 90);
    assert!(
        body["recovery_date"].is_null(),
        "a wallet that never runs out has no recovery date: {body}"
    );
    assert!(
        body["trough_minor"].as_i64().unwrap() > 0,
        "the balance never goes negative: {body}"
    );
    assert!(
        body["projected_balance_minor"].as_i64().unwrap() > 0,
        "it is still solvent at the end of the horizon: {body}"
    );
}

/// A balance that is already negative projects forward: the answer is how
/// deep the hole gets and when it recovers (D38).
#[tokio::test]
async fn a_negative_balance_projects_forward() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    // Overdrawn by 5000, spending 1000 a day.
    let wallet = wallet_with_spending(&app, &pool, user, -5000, 1000).await;

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK, "runway failed: {body}");

    assert_eq!(
        body["starting_balance_minor"], -5000 - 14_000,
        "the projection starts from the ledger's balance, which the recorded spending has already debited"
    );
    assert!(
        body["trough_minor"].as_i64().unwrap() < -5000,
        "the hole gets deeper: {body}"
    );
    assert!(
        body["recovery_date"].is_null(),
        "with no income, it never recovers: {body}"
    );
}

/// A paycheck inside the horizon brings the balance back: the recovery date
/// is when it happens.
#[tokio::test]
async fn a_scheduled_income_produces_a_recovery_date() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    // Overdrawn, spending 1000 a day.
    let wallet = wallet_with_spending(&app, &pool, user, -5000, 1000).await;

    // A paycheck in two weeks.
    let payday = chrono::Utc::now().date_naive() + chrono::Duration::days(14);
    let (status, income) = post(
        &app,
        user,
        "/income",
        json!({
            "name": "Salary",
            "amount_minor": 100_000,
            "currency": "USD",
            "cadence": "monthly",
            "starts_on": payday.to_string(),
            "allocations": [{"wallet_id": wallet, "amount_minor": 100_000}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "income failed: {income}");

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK, "runway failed: {body}");

    let recovery = body["recovery_date"]
        .as_str()
        .expect("a recovery date: the paycheck covers the hole");
    let recovery = chrono::NaiveDate::parse_from_str(recovery, "%Y-%m-%d")
        .expect("date")
        .to_string();
    assert_eq!(
        recovery,
        payday.to_string(),
        "the recovery is the payday"
    );
    assert!(
        body["projected_balance_minor"].as_i64().unwrap() > 0,
        "and the balance is positive at the end of the horizon: {body}"
    );
}

/// A recurring charge is projected as a dated movement, not left inside the
/// pace: it is subtracted once, on its day.
#[tokio::test]
async fn a_recurring_charge_is_subtracted_on_its_day() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = wallet_with_spending(&app, &pool, user, 100_000, 1000).await;

    // A weekly charge of 10000, starting today.
    let today = chrono::Utc::now().date_naive();
    let (status, rule) = post(
        &app,
        user,
        "/recurring",
        json!({
            "wallet_id": wallet,
            "name": "Rent",
            "amount_minor": 10_000,
            "currency": "USD",
            "cadence": "weekly",
            "starts_on": today.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "rule failed: {rule}");

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK, "runway failed: {body}");

    // 90 days at 1000 a day is 89000 of discretionary spend, plus 12 weekly
    // charges of 10000 is 120000. The starting balance is 100000 less the
    // 14000 already spent. The balance must reflect the charge exactly once.
    let expected = 100_000 - 14_000 - 89_000 - 120_000;
    assert_eq!(
        body["projected_balance_minor"], expected,
        "the charge is subtracted once, on its day: {body}"
    );
}

/// The assumptions travel with the answer, so a stale projection explains
/// itself.
#[tokio::test]
async fn the_projection_carries_its_assumptions() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = wallet_with_spending(&app, &pool, user, 100_000, 1000).await;

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK);

    let assumptions = body["assumptions"].as_array().expect("array");
    assert!(!assumptions.is_empty(), "a forecast must state its assumptions");
    assert!(
        assumptions
            .iter()
            .any(|a| a.as_str().unwrap_or_default().contains("horizon")),
        "the horizon must be stated: {body}"
    );
    assert!(
        body["as_of"].as_str().is_some(),
        "the projection must say when it was computed: {body}"
    );
}

/// A cancelled rule stops being forecast, so its future charges leave the
/// projection. Cancelling must not make the forecast more pessimistic.
#[tokio::test]
async fn a_cancelled_rule_is_not_projected_as_a_future_charge() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = wallet_with_spending(&app, &pool, user, 100_000, 1000).await;

    let today = chrono::Utc::now().date_naive();
    let (status, rule) = post(
        &app,
        user,
        "/recurring",
        json!({
            "wallet_id": wallet,
            "name": "Rent",
            "amount_minor": 10_000,
            "currency": "USD",
            "cadence": "weekly",
            "starts_on": today.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "rule failed: {rule}");
    let rule_id = rule["id"].as_str().expect("rule id");

    let (_, with_rule) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    let charged = with_rule["projected_balance_minor"].as_i64().unwrap();

    let (status, _) = post(&app, user, &format!("/recurring/{rule_id}/cancel"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "cancel failed");

    let (_, cancelled) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(
        cancelled["projected_balance_minor"].as_i64().unwrap(),
        charged + 120_000,
        "the twelve weekly charges inside the horizon are no longer projected"
    );
}

/// With too little history the projection has no pace, and says so rather than
/// passing a zero pace off as a measurement (D42).
#[tokio::test]
async fn a_projection_without_a_burn_rate_says_so() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    purchase_on(&app, user, wallet, 1000, 0).await;
    project(&pool).await;

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK, "runway failed: {body}");

    assert!(
        body["burn_rate"]["daily_minor"].is_null(),
        "no pace is measured: {body}"
    );
    assert!(
        body["assumptions"]
            .as_array()
            .expect("array")
            .iter()
            .any(|a| a.as_str().unwrap_or_default().contains("no burn rate")),
        "the projection must say the pace is missing: {body}"
    );
}

/// One user's wallet is not another's.
#[tokio::test]
async fn another_users_wallet_has_no_runway() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = wallet_with_spending(&app, &pool, owner, 100_000, 1000).await;

    let (status, _) = get(&app, intruder, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "another user's wallet must have no runway"
    );
}

/// A charge due today is not projected.
///
/// The projection starts from the balance now and simulates from tomorrow. A
/// charge landing on today may already be IN that balance, and the projection
/// cannot tell, so it is never generated at all (D61): nothing is dropped and
/// nothing can be double-counted.
///
/// A yearly rule starting today fires today and not again within the horizon,
/// so it is the cleanest way to isolate the boundary.
#[tokio::test]
async fn a_charge_due_today_is_not_projected() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    // A fortnight of spending so there is a pace to project.
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 1000, days_ago).await;
    }
    project(&pool).await;

    let today = chrono::Utc::now().date_naive();
    let (status, rule) = post(
        &app,
        user,
        "/recurring",
        json!({
            "wallet_id": wallet,
            "name": "Annual fee",
            "amount_minor": 50_000,
            "currency": "USD",
            "cadence": "yearly",
            "starts_on": today.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "rule failed: {rule}");

    let (status, body) = get(&app, user, &format!("/analytics/runway/{wallet}")).await;
    assert_eq!(status, StatusCode::OK, "runway failed: {body}");

    // The 50_000 charge lands on today and is not projected. The projection
    // is the discretionary pace alone.
    let assumptions = body["assumptions"].as_array().expect("array");
    assert!(
        assumptions.iter().any(|a| {
            a.as_str()
                .unwrap_or_default()
                .contains("due today is not projected")
        }),
        "the boundary must be stated: {body}"
    );

    // A yearly charge 50_000 due today would, if projected, drop the balance
    // by 50_000 more than this. The projection is the pace alone.
    let daily = body["burn_rate"]["daily_minor"].as_i64().expect("a pace");
    let expected_end = body["starting_balance_minor"].as_i64().unwrap()
        - daily * (90 - 1);
    assert_eq!(
        body["projected_balance_minor"], expected_end,
        "only the discretionary pace is projected: {body}"
    );
}
