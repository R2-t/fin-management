//! The currency reference data must be correct and consistent: the SQL table and
//! the Rust formatting table describe the same currencies, and both must match
//! the ISO-4217 exponents written out by hand below.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use financial_management::application::currency::minor_unit;

/// Exponents taken directly from ISO-4217, not from either implementation, so
/// this test can actually disagree with the code.
const ISO_4217_EXPONENTS: &[(&str, u8)] = &[
    ("USD", 2),
    ("EUR", 2),
    ("GBP", 2),
    ("JPY", 0),
    ("KRW", 0),
    ("CLP", 0),
    ("ISK", 0),
    ("KWD", 3),
    ("BHD", 3),
    ("TND", 3),
];

#[tokio::test]
async fn reference_table_matches_iso_4217_and_the_rust_table() {
    let pool = test_pool().await;
    let conn = pool.get().await.expect("pool checkout");

    for (code, expected) in ISO_4217_EXPONENTS {
        let row = conn
            .query_opt(
                "SELECT minor_unit FROM currency WHERE code = $1",
                &[code],
            )
            .await
            .expect("currency query failed")
            .unwrap_or_else(|| panic!("{code} missing from the currency reference table"));
        let from_db: i16 = row.get("minor_unit");

        assert_eq!(
            from_db as u8, *expected,
            "{code}: the reference table disagrees with ISO-4217"
        );
        assert_eq!(
            minor_unit(code),
            *expected,
            "{code}: the Rust formatting table disagrees with ISO-4217"
        );
    }
}

/// The two tables must list the same currencies, so a currency cannot be
/// formattable in Rust while being unknown to SQL (or the reverse).
#[tokio::test]
async fn sql_and_rust_tables_agree_row_for_row() {
    let pool = test_pool().await;
    let conn = pool.get().await.expect("pool checkout");

    let rows = conn
        .query("SELECT code, minor_unit FROM currency ORDER BY code", &[])
        .await
        .expect("currency query failed");

    for row in &rows {
        let code: String = row.get("code");
        let from_db: i16 = row.get("minor_unit");
        assert_eq!(
            minor_unit(&code) as i16,
            from_db,
            "{code}: Rust and SQL disagree; update both in the same change"
        );
    }
    assert!(!rows.is_empty(), "the reference table must not be empty");
}

/// A client must be able to render money correctly for a zero-decimal currency,
/// which it cannot do from an integer amount alone.
#[tokio::test]
async fn responses_expose_the_currencys_exponent() {
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let yen = create_wallet(&app, user, "jpy").await;
    let (_, wallet) = get(&app, user, "/wallet").await;
    let jpy_wallet = wallet
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == yen.to_string())
        .expect("created wallet present");
    assert_eq!(
        jpy_wallet["currency_minor_unit"], 0,
        "JPY has no decimal places, so a client must not render 10.00"
    );

    let dollars = create_wallet(&app, user, "usd").await;
    let usd_purchase = create_purchase(&app, user, dollars, "USD", 1050).await;
    let (_, purchase) = get(&app, user, &format!("/purchase/{usd_purchase}")).await;
    assert_eq!(purchase["currency_minor_unit"], 2);
    assert_eq!(purchase["currency"], "USD");
}

/// Converting between currencies with different decimal places must account for
/// the exponent difference, not just the exchange rate.
///
/// $10.00 is 1000 minor units. At 150 JPY per USD that is ¥1500, i.e. 1500 minor
/// units in JPY (which has no decimals). Multiplying the minor-unit amount by the
/// major-unit rate alone gives 150000 -- wrong by the exponent difference (100x).
#[tokio::test]
async fn conversion_accounts_for_the_minor_unit_difference() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let yen_wallet = create_wallet(&app, user, "jpy").await;
    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": yen_wallet,
            "store": "Tokyo",
            "currency": "USD",
            "exchange_rate_override": 150.0,
            "items": [{"name": "thing", "unit_price": 1000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create failed: {body}");
    let purchase_id = body["id"].as_str().unwrap().parse().unwrap();
    project(&pool).await;

    let (amount, rate, converted, resolution) = fact_row(&pool, purchase_id).await.expect("fact row");
    assert_eq!(amount, 1000, "USD cents");
    assert_eq!(rate, Some(150_000_000_000), "150 JPY per USD, scaled");
    assert_eq!(converted, Some(1500), "1000 USD cents at 150 JPY/USD is 1500 yen");
    assert_eq!(resolution, "completed");
}
