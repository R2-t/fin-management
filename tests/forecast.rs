//! Integration tests for the overspend forecast.
//!
//! The projection answers: given what has been spent so far and the pace,
//! will the budget go over its limit before the period ends?

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

/// A monthly budget of `amount_minor`, starting this month.
async fn monthly_budget(
    app: &axum::Router,
    user: uuid::Uuid,
    wallet: uuid::Uuid,
    amount_minor: i64,
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
            "starts_on": first_of_month.to_string()
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "budget create failed: {body}");
    body["id"].as_str().expect("budget id").parse().expect("uuid")
}

/// A budget that is comfortably within its limit is not projected to
/// overspend.
#[tokio::test]
async fn a_budget_within_its_limit_is_not_projected_to_overspend() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    // A fortnight of modest spending.
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 1000, days_ago).await;
    }
    project(&pool).await;

    // A limit far above the pace: 1000 a day for 30 days is 30000.
    let budget = monthly_budget(&app, user, wallet, 100_000).await;

    // The period must exist before it can be forecast.
    get(&app, user, &format!("/budget/{budget}/status")).await;

    let (status, body) = get(&app, user, &format!("/forecast/budget/{budget}/overspend")).await;
    assert_eq!(status, StatusCode::OK, "forecast failed: {body}");

    assert_eq!(body["will_overspend"], false, "{body}");
    assert_eq!(body["overspend_minor"], 0);
    assert!(body["overspend_date"].is_null());
    assert_eq!(body["limit_minor"], 100_000);
    assert_eq!(body["currency"], "USD");
    assert!(
        !body["assumptions"].as_array().expect("array").is_empty(),
        "a forecast must state its assumptions: {body}"
    );
}

/// A budget on pace to overspend says so, and says by how much and when.
#[tokio::test]
async fn a_budget_on_pace_to_overspend_is_projected_to_overspend() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 10_000, days_ago).await;
    }
    project(&pool).await;

    // 10000 a day for 30 days is 300000, against a 100000 limit.
    let budget = monthly_budget(&app, user, wallet, 100_000).await;
    get(&app, user, &format!("/budget/{budget}/status")).await;

    let (status, body) = get(&app, user, &format!("/forecast/budget/{budget}/overspend")).await;
    assert_eq!(status, StatusCode::OK, "forecast failed: {body}");

    assert_eq!(body["will_overspend"], true, "{body}");
    assert!(
        body["overspend_minor"].as_i64().unwrap() > 0,
        "it must say by how much: {body}"
    );
    assert!(
        !body["overspend_date"].is_null(),
        "it must say when: {body}"
    );
    let overspend_date = body["overspend_date"].as_str().expect("a date");
    let today = chrono::Utc::now().date_naive().to_string();
    assert!(
        overspend_date >= today.as_str(),
        "the crossing must be in the future: {body}"
    );
}

/// The forecast is persisted with its assumptions, so it can be compared and
/// back-tested later.
#[tokio::test]
async fn the_forecast_is_persisted_with_its_assumptions() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 1000, days_ago).await;
    }
    project(&pool).await;

    let budget = monthly_budget(&app, user, wallet, 100_000).await;
    get(&app, user, &format!("/budget/{budget}/status")).await;

    let (status, _) = get(&app, user, &format!("/forecast/budget/{budget}/overspend")).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = get(&app, user, "/forecast").await;
    assert_eq!(status, StatusCode::OK, "list failed: {body}");

    let forecasts = body.as_array().expect("array");
    assert_eq!(forecasts.len(), 1, "the forecast is persisted: {body}");

    let forecast = &forecasts[0];
    assert_eq!(forecast["kind"], "budget_overspend");
    assert_eq!(forecast["model_version"], "runway-v1");
    assert!(!forecast["assumptions"].as_array().expect("array").is_empty());
    assert!(forecast["result"]["will_overspend"].is_boolean());
    assert_eq!(forecast["horizon_days"], forecast["result"]["days_remaining"]);
}

/// The list can be filtered by kind.
#[tokio::test]
async fn the_forecast_list_can_be_filtered_by_kind() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    for days_ago in 1..=14 {
        purchase_on(&app, user, wallet, 1000, days_ago).await;
    }
    project(&pool).await;

    let budget = monthly_budget(&app, user, wallet, 100_000).await;
    get(&app, user, &format!("/budget/{budget}/status")).await;
    get(&app, user, &format!("/forecast/budget/{budget}/overspend")).await;

    let (status, body) = get(&app, user, "/forecast?kind=budget_overspend").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 1);

    let (status, body) = get(&app, user, "/forecast?kind=cash_runway").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().expect("array").len(), 0);

    let (status, body) = get(&app, user, "/forecast?kind=nonsense").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// One user's forecasts are not another's.
#[tokio::test]
async fn another_users_forecasts_are_not_visible() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;
    for days_ago in 1..=14 {
        purchase_on(&app, owner, wallet, 1000, days_ago).await;
    }
    project(&pool).await;

    let budget = monthly_budget(&app, owner, wallet, 100_000).await;
    get(&app, owner, &format!("/budget/{budget}/status")).await;
    get(&app, owner, &format!("/forecast/budget/{budget}/overspend")).await;

    let (status, body) = get(&app, intruder, "/forecast").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.as_array().expect("array").len(),
        0,
        "another user's forecasts must not be visible"
    );
}

/// A budget whose period has not been materialised cannot be forecast: the
/// forecast needs the period's effective limit.
#[tokio::test]
async fn a_budget_without_a_materialised_period_cannot_be_forecast() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let budget = monthly_budget(&app, user, wallet, 100_000).await;

    let (status, _) = get(&app, user, &format!("/forecast/budget/{budget}/overspend")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the forecast needs the period's effective limit"
    );
}

/// The forecast prices the remaining days by their weekday, not by an
/// average.
///
/// Maria spends 1000 a day on weekdays and nothing at weekends. A flat pace
/// of 1000 projects the whole period at 1000 a day; the profile prices each
/// remaining day by what her own history says that weekday costs. The two
/// disagree whenever the remaining days are not a whole number of identical
/// weeks.
#[tokio::test]
async fn the_forecast_prices_the_remaining_days_by_their_weekday() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    // Two weeks: 1000 on weekdays, nothing on weekends.
    for days_ago in 1..=14 {
        let date = chrono::Utc::now() - chrono::Duration::days(days_ago);
        let price = match date.weekday() {
            chrono::Weekday::Sat | chrono::Weekday::Sun => 0,
            _ => 1000,
        };
        if price > 0 {
            purchase_on(&app, user, wallet, price, days_ago).await;
        }
    }
    project(&pool).await;

    let budget = monthly_budget(&app, user, wallet, 100_000).await;
    get(&app, user, &format!("/budget/{budget}/status")).await;

    let (status, body) = get(&app, user, &format!("/forecast/budget/{budget}/overspend")).await;
    assert_eq!(status, StatusCode::OK, "forecast failed: {body}");

    // The pace is the profile's average, but the projection is NOT the pace
    // times the days: it prices each remaining day by its weekday.
    assert_eq!(body["daily_minor"].as_i64().unwrap(), 714, "{body}");

    let projected: i64 = body["projected_spend_minor"].as_i64().unwrap();
    let flat = body["daily_minor"].as_i64().unwrap()
        * body["days_remaining"].as_i64().unwrap();
    assert_ne!(
        projected, flat,
        "a weekend-shaped history must not project like a flat one: {body}"
    );

    // And it is the sum of the remaining days priced by their weekday.
    let today = chrono::Utc::now().date_naive();
    let days_remaining = body["days_remaining"].as_i64().unwrap();
    let mut expected = 0i64;
    let mut date = today;
    for _ in 0..days_remaining {
        expected += match date.weekday() {
            chrono::Weekday::Sat | chrono::Weekday::Sun => 0,
            _ => 1000,
        };
        date += chrono::Duration::days(1);
    }
    assert_eq!(projected, expected, "each remaining day priced by its weekday: {body}");
}
