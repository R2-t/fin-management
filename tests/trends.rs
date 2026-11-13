//! Integration tests for the trends read model (Phase 1).
//!
//! Seam: the HTTP surface, driven in-process against a real TimescaleDB. The
//! bucketing assertions are the point of these tests: a "day" or "month" must be
//! the user's day or month, not UTC's.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// Sets the user's timezone through the profile API.
async fn set_timezone(app: &axum::Router, user: uuid::Uuid, tz: &str) {
    let (status, body) = put(app, user, "/profile", json!({ "timezone": tz })).await;
    assert_eq!(status, StatusCode::OK, "profile update failed: {body}");
}

fn trends_url(bucket: &str, from: &str, to: &str) -> String {
    format!("/analytics/trends?bucket={bucket}&from={from}&to={to}")
}

/// A purchase near midnight must land in the month the *user* was in.
#[tokio::test]
async fn buckets_use_the_users_timezone() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    set_timezone(&app, user, "Europe/Madrid").await;
    let wallet = create_wallet(&app, user, "eur").await;

    // 23:30 UTC on 30 September is 01:30 on 1 October in Madrid (UTC+2).
    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Midnight", "currency": "EUR",
            "purchase_date": "2026-09-30T23:30:00Z",
            "items": [{"name": "x", "unit_price": 1000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    project(&pool).await;

    let (status, body) = get(
        &app,
        user,
        &trends_url("month", "2026-09-01T00:00:00Z", "2026-11-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "trends failed: {body}");

    let series = body["series"].as_array().expect("series present");
    let eur = series
        .iter()
        .find(|s| s["currency"] == "EUR")
        .expect("EUR series");
    let starts: Vec<&str> = eur["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["bucket_start"].as_str().unwrap())
        .collect();

    assert_eq!(starts.len(), 1, "one month bucket, not two: {starts:?}");

    // The bucket start is a UTC instant, so it must be read back in the user's
    // zone to be labelled. In Madrid this instant is 1 October 00:00, while a
    // UTC-bucketed series would have placed the purchase in September.
    let start: chrono::DateTime<chrono::Utc> = starts[0].parse().unwrap();
    let local = chrono_tz::Europe::Madrid.from_utc_datetime(&start.naive_utc());
    use chrono::{Datelike, TimeZone, Timelike};
    assert_eq!(
        (local.year(), local.month(), local.day(), local.hour()),
        (2026, 10, 1, 0),
        "expected the Madrid October bucket, got {local} (raw {start})"
    );
    assert_eq!(body["timezone"], "Europe/Madrid");
    assert!(body["as_of"].is_string(), "the answer carries its as_of");
}

/// Totals are per-currency integers, and an unresolved conversion contributes
/// nothing rather than a fabricated amount.
#[tokio::test]
async fn totals_are_integers_and_exclude_unresolved_conversions() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    execute(&pool, "DELETE FROM exchange_rate_cache WHERE from_currency='USD' AND to_currency='EUR'").await;
    let wallet = create_wallet(&app, user, "eur").await;

    // A same-currency purchase: 12.34 EUR.
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Known", "currency": "EUR",
            "items": [{"name": "x", "unit_price": 1234, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(body["currency"], "EUR");

    // A USD purchase into the EUR wallet with no rate available: recorded, not converted.
    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Unknown", "currency": "USD",
            "items": [{"name": "y", "unit_price": 5000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    assert_eq!(body["rate_source"], "unresolved");
    project(&pool).await;

    let (_, body) = get(
        &app,
        user,
        &trends_url("month", "2020-01-01T00:00:00Z", "2030-01-01T00:00:00Z"),
    )
    .await;
    let eur = body["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["currency"] == "EUR")
        .expect("EUR series");
    let point = &eur["points"].as_array().unwrap()[0];

    assert_eq!(
        point["total_minor"].as_i64().unwrap(),
        1234,
        "the unresolved USD purchase must not contribute to the EUR total"
    );
    assert!(point["total_minor"].is_i64(), "money is an integer, never a float");
    assert_eq!(point["purchases"], 1, "only the converted purchase counts");
    assert_eq!(point["unresolved"], 1, "the unresolved one is still reported");
}

/// Month-over-month growth compares each month with the one before it.
#[tokio::test]
async fn month_over_month_growth_is_reported_per_currency() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "eur").await;
    for (date, price) in [("2026-01-15T12:00:00Z", 10000i64), ("2026-02-15T12:00:00Z", 12500)] {
        let (status, body) = post(
            &app,
            user,
            "/purchase/purchase_with_items",
            json!({
                "wallet_id": wallet, "store": "Monthly", "currency": "EUR",
                "purchase_date": date,
                "items": [{"name": "x", "unit_price": price, "quantity": 1}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    }
    project(&pool).await;

    let (_, body) = get(
        &app,
        user,
        &trends_url("month", "2026-01-01T00:00:00Z", "2026-03-01T00:00:00Z"),
    )
    .await;
    let points = body["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["currency"] == "EUR")
        .unwrap()["points"]
        .as_array()
        .unwrap()
        .clone();

    assert_eq!(points.len(), 2, "two month buckets");
    assert!(points[0]["growth"].is_null(), "the first month has no predecessor");
    let growth = &points[1]["growth"];
    assert_eq!(growth["previous_total"], 10000);
    assert_eq!(growth["delta_minor"], 2500);
    assert_eq!(growth["delta_pct"], 25.0);
}

/// A rolling average is money and therefore an integer, and it averages the
/// days in the window rather than the purchases.
#[tokio::test]
async fn rolling_average_is_an_integer_over_the_window() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "eur").await;
    // 3 days: 100, 200, 300 minor units. A 3-day window averages all three.
    for (date, price) in [
        ("2026-03-02T12:00:00Z", 100i64),
        ("2026-03-03T12:00:00Z", 200),
        ("2026-03-04T12:00:00Z", 300),
    ] {
        let (status, body) = post(
            &app,
            user,
            "/purchase/purchase_with_items",
            json!({
                "wallet_id": wallet, "store": "Daily", "currency": "EUR",
                "purchase_date": date,
                "items": [{"name": "x", "unit_price": price, "quantity": 1}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    }
    project(&pool).await;

    let (status, body) = get(
        &app,
        user,
        "/analytics/rolling?window_days=3&from=2026-03-01T00:00:00Z&to=2026-03-06T00:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rolling failed: {body}");
    assert_eq!(body["window_days"], 3);

    let points = body["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["currency"] == "EUR")
        .unwrap()["points"]
        .as_array()
        .unwrap()
        .clone();

    // The series starts on the range's first day (1 March, no spend) and the
    // window averages calendar days. With a 3-day window, 4 March averages
    // 2-4 March: 1 March has fallen out of the window.
    // The series runs the whole requested range, 1-5 March inclusive, not
    // just the days with spend: an average over N days must count the empty
    // days as zero at both ends.
    assert_eq!(points.len(), 5, "1-5 March inclusive: {points:?}");
    assert_eq!(points[0]["day"], "2026-03-01");
    assert_eq!(points[0]["total_minor"], 0);
    assert_eq!(points[0]["avg_minor"], 0, "0/1");
    assert_eq!(points[1]["avg_minor"], 50, "(0+100)/2");
    assert_eq!(points[2]["avg_minor"], 100, "(0+100+200)/3");
    assert_eq!(points[3]["avg_minor"], 200, "(100+200+300)/3 - 1 March is out of the 3-day window");
    assert_eq!(points[4]["day"], "2026-03-05");
    assert_eq!(points[4]["total_minor"], 0, "5 March is in the range and had no spend");
    assert_eq!(points[4]["avg_minor"], 167, "(200+300+0)/3");
    assert!(points[3]["avg_minor"].is_i64(), "money stays an integer");
}

/// One user's trends must not include another's spending.
#[tokio::test]
async fn trends_are_scoped_to_the_requesting_user() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let stranger = new_user();

    let wallet = create_wallet(&app, owner, "eur").await;
    create_purchase(&app, owner, wallet, "EUR", 9999).await;
    project(&pool).await;

    let (status, body) = get(
        &app,
        stranger,
        &trends_url("month", "2020-01-01T00:00:00Z", "2030-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["series"].as_array().unwrap().is_empty()
            || body["series"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["points"].as_array().unwrap().is_empty()),
        "a stranger sees no spending: {body}"
    );
}

/// A bad bucket or an inverted range is the caller's mistake, not a 500.
#[tokio::test]
async fn invalid_queries_are_rejected() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let (status, _) = get(
        &app,
        user,
        &trends_url("fortnight", "2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown bucket");

    let (status, _) = get(
        &app,
        user,
        &trends_url("month", "2026-03-01T00:00:00Z", "2026-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "range ends before it starts");
}

/// Correcting a Purchase's date must move it between buckets, and deleting one
/// must remove it: the projection is a projection, not a second ledger.
#[tokio::test]
async fn edits_and_deletes_are_reflected_in_trends() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "eur").await;
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Movable", "currency": "EUR",
            "purchase_date": "2026-01-15T12:00:00Z",
            "items": [{"name": "x", "unit_price": 5000, "quantity": 1}]
        }),
    )
    .await;
    let purchase_id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();
    project(&pool).await;

    // Move it into February.
    let (status, _) = put(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": 1, "purchase_date": "2026-02-15T12:00:00Z"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "correcting the date must be accepted");
    project(&pool).await;

    let (_, body) = get(
        &app,
        user,
        &trends_url("month", "2026-01-01T00:00:00Z", "2026-04-01T00:00:00Z"),
    )
    .await;
    let points = body["series"].as_array().unwrap()[0]["points"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(points.len(), 1, "only the new month has spend: {points:?}");
    let start: chrono::DateTime<chrono::Utc> =
        points[0]["bucket_start"].as_str().unwrap().parse().unwrap();
    assert!(
        start.to_string().starts_with("2026-02"),
        "the purchase moved to February, got {start}"
    );

    // Delete it; the bucket disappears.
    let version = 2;
    let (status, _) = delete(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": version}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete failed");
    project(&pool).await;

    let (_, body) = get(
        &app,
        user,
        &trends_url("month", "2026-01-01T00:00:00Z", "2026-04-01T00:00:00Z"),
    )
    .await;
    assert!(
        body["series"].as_array().unwrap().is_empty()
            || body["series"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["points"].as_array().unwrap().is_empty()),
        "a deleted purchase leaves no trend: {body}"
    );
}

/// An uncategorized Purchase is still spend: the category filter excludes it on
/// request, but an unfiltered series includes it.
#[tokio::test]
async fn uncategorized_purchases_are_included_unless_filtered() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "eur").await;
    // No category_id supplied.
    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Unsorted", "currency": "EUR",
            "items": [{"name": "x", "unit_price": 2500, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    assert!(body["category_id"].is_null(), "recorded without a category");
    project(&pool).await;

    let (_, all) = get(
        &app,
        user,
        &trends_url("month", "2020-01-01T00:00:00Z", "2030-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(
        all["series"].as_array().unwrap()[0]["points"]
            .as_array()
            .unwrap()[0]["total_minor"],
        2500,
        "an uncategorized purchase is still spend: {all}"
    );

    let (_, filtered) = get(
        &app,
        user,
        &format!(
            "{}&category_id=1",
            trends_url("month", "2020-01-01T00:00:00Z", "2030-01-01T00:00:00Z")
        ),
    )
    .await;
    assert!(
        filtered["series"].as_array().unwrap().is_empty()
            || filtered["series"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["points"].as_array().unwrap().is_empty()),
        "filtering by a category excludes uncategorized spend: {filtered}"
    );
}

/// A "day" in the rolling series must be the user's calendar day, and the window
/// must average over calendar days — including days with no spend. A sparse
/// window that averaged only the days present would overstate the average, and a
/// server-timezone label would put a late-evening purchase on the wrong date.
#[tokio::test]
async fn rolling_uses_the_users_calendar_days_including_empty_ones() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    set_timezone(&app, user, "Europe/Madrid").await;
    let wallet = create_wallet(&app, user, "eur").await;

    // 23:30 UTC on 1 March is 00:30 on 2 March in Madrid.
    for (date, price) in [("2026-03-01T23:30:00Z", 100i64), ("2026-03-04T12:00:00Z", 300)] {
        let (status, body) = post(
            &app,
            user,
            "/purchase/purchase_with_items",
            json!({
                "wallet_id": wallet, "store": "Sparse", "currency": "EUR",
                "purchase_date": date,
                "items": [{"name": "x", "unit_price": price, "quantity": 1}]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    }
    project(&pool).await;

    let (status, body) = get(
        &app,
        user,
        "/analytics/rolling?window_days=4&from=2026-03-01T00:00:00Z&to=2026-03-06T00:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rolling failed: {body}");

    let points = body["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["currency"] == "EUR")
        .expect("EUR series")["points"]
        .as_array()
        .unwrap()
        .clone();

    // Madrid days present: 2 Mar (100) and 4 Mar (300). The window spans calendar
    // days, so 2 Mar and 3 Mar average over two days (50), not one (100).
    let by_day: Vec<(String, i64, i64)> = points
        .iter()
        .map(|p| {
            (
                p["day"].as_str().unwrap().to_string(),
                p["total_minor"].as_i64().unwrap(),
                p["avg_minor"].as_i64().unwrap(),
            )
        })
        .collect();

    assert_eq!(
        by_day,
        vec![
            ("2026-03-01".to_string(), 0, 0),
            ("2026-03-02".to_string(), 100, 50),
            ("2026-03-03".to_string(), 0, 33),
            ("2026-03-04".to_string(), 300, 100),
            // 5 March is in the requested range and had no spend, so it is a
            // zero day in the series rather than the end of it.
            ("2026-03-05".to_string(), 0, 100),
        ],
        "days are local, the series starts at the range start, and empty days \
         count as zero spend: {by_day:?}"
    );
}
