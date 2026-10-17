//! Database-backed integration tests for the record → project → correct path.
//!
//! These drive the HTTP router in-process and assert on real database state.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

/// A cross-currency Purchase whose rate cannot be resolved must still be
/// recorded, and must never be given an invented 1:1 conversion.
#[tokio::test]
async fn foreign_currency_purchase_without_rate_is_recorded_as_unresolved() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    // Wallet holds EUR; the Purchase is paid in USD. The mock provider knows no
    // USD/EUR pair, so no rate can be resolved.
    // Arrange the precondition explicitly: the rate cache is shared across tests
    // (and runs), so a pair left there by another test would make this purchase
    // resolve at record time.
    execute(&pool, "DELETE FROM exchange_rate_cache WHERE from_currency='USD' AND to_currency='EUR'").await;
    let wallet = create_wallet(&app, user, "eur").await;
    let (status, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet,
            "store": "Abroad",
            "currency": "USD",
            "items": [{"name": "thing", "unit_price": 1000, "quantity": 1}]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED, "recording must not fail: {body}");
    assert_eq!(body["rate_source"], "unresolved");
    let purchase_id = body["id"].as_str().unwrap().parse().unwrap();

    project(&pool).await;

    let (amount, rate, converted, resolution) = fact_row(&pool, purchase_id).await.expect("fact row");
    assert_eq!(amount, 1000);
    assert_eq!(rate, None, "no rate should be stored");
    assert_eq!(
        converted, None,
        "an unresolved conversion must not be recorded at parity"
    );
    assert_eq!(resolution, "unresolved");
}

/// Completing a rate must reach the projection and flip it to completed.
#[tokio::test]
async fn completing_a_rate_reaches_the_projection() {
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
            "wallet_id": wallet, "store": "Abroad", "currency": "USD",
            "items": [{"name": "thing", "unit_price": 1000, "quantity": 1}]
        }),
    )
    .await;
    let purchase_id = body["id"].as_str().unwrap().parse().unwrap();
    let version = body["version"].as_i64().unwrap();
    project(&pool).await;

    let (status, updated) = put(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": version, "exchange_rate": 0.85}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rate completion failed: {updated}");
    project(&pool).await;

    let (amount, rate, converted, resolution) = fact_row(&pool, purchase_id).await.expect("fact row");
    assert_eq!(amount, 1000);
    assert_eq!(rate, Some(850_000_000), "stored rate is the scaled integer");
    assert_eq!(converted, Some(850), "1000 minor units at 0.85");
    assert_eq!(resolution, "completed");
}

/// Deleting a Purchase must remove its projected row rather than leaving stale
/// analytics behind.
#[tokio::test]
async fn deleting_a_purchase_removes_its_projection() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Gone", "currency": "USD",
            "items": [{"name": "thing", "unit_price": 500, "quantity": 1}]
        }),
    )
    .await;
    let purchase_id = body["id"].as_str().unwrap().parse().unwrap();
    let version = body["version"].as_i64().unwrap();
    project(&pool).await;
    assert_eq!(count_fact_rows(&pool, purchase_id).await, 1);

    let (status, _) = delete(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": version}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    project(&pool).await;

    assert_eq!(
        count_fact_rows(&pool, purchase_id).await,
        0,
        "soft delete must remove the derived row"
    );
}

/// Re-projecting the same Purchase must converge on identical state, so
/// duplicate or replayed outbox work cannot corrupt analytics.
#[tokio::test]
async fn replayed_projection_is_idempotent() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Repeat", "currency": "USD",
            "items": [{"name": "thing", "unit_price": 750, "quantity": 2}]
        }),
    )
    .await;
    let purchase_id = body["id"].as_str().unwrap().parse().unwrap();

    project(&pool).await;
    let first = fact_row(&pool, purchase_id).await.expect("fact row");

    // Replaying projection work must not duplicate or alter the row.
    for _ in 0..3 {
        project(&pool).await;
    }

    assert_eq!(count_fact_rows(&pool, purchase_id).await, 1, "one row per Purchase");
    assert_eq!(fact_row(&pool, purchase_id).await.expect("fact row"), first);
}

/// Reconciliation must have teeth: a corrupted stored amount is detected. This
/// is the check the earlier review claimed was tautological; it is not.
#[tokio::test]
async fn corrupted_amount_is_detected_by_reconciliation() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let purchase_id = create_purchase(&app, user, wallet, "USD", 1000).await;
    project(&pool).await;

    assert_eq!(
        mismatches_for(&pool, user).await,
        0,
        "a correct projection must reconcile cleanly"
    );

    let conn = pool.get().await.unwrap();
    conn.execute(
        "UPDATE purchase_fact SET amount_base_minor = amount_base_minor + 1 WHERE purchase_id = $1",
        &[&purchase_id],
    )
    .await
    .expect("corrupting the stored amount");

    assert_eq!(
        mismatches_for(&pool, user).await,
        1,
        "reconciliation must detect the corrupted amount"
    );
}

/// A missing record and a stale version must be distinguishable by a client.
#[tokio::test]
async fn missing_record_is_not_found_and_stale_version_is_conflict() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let (status, _) = get(&app, user, &format!("/purchase/{}", uuid::Uuid::new_v4())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let wallet = create_wallet(&app, user, "usd").await;
    let purchase_id = create_purchase(&app, user, wallet, "USD", 100).await;

    let (stale, _) = put(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": 99, "store": "Renamed"}),
    )
    .await;
    assert_eq!(
        stale,
        StatusCode::CONFLICT,
        "a stale optimistic-lock version must not read as a server error"
    );

    let (ok, _) = put(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": 1, "store": "Renamed"}),
    )
    .await;
    assert_eq!(ok, StatusCode::OK);
}

/// Pagination must be bounded and must report the true total.
#[tokio::test]
async fn pagination_is_bounded_and_reports_total() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    for i in 0..3 {
        create_purchase(&app, user, wallet, "USD", 100 + i).await;
    }

    let (status, body) = get(&app, user, "/purchase?limit=9999").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["limit"], 200, "an oversized limit must be clamped");
    assert!(body["total"].as_i64().unwrap() >= 3);

    let (_, page) = get(&app, user, "/purchase?limit=2").await;
    assert_eq!(page["data"].as_array().unwrap().len(), 2);
    assert!(page["total"].as_i64().unwrap() >= 3, "total ignores the page size");

    let (_, negative) = get(&app, user, "/purchase?offset=-5").await;
    assert_eq!(negative["offset"], 0, "a negative offset must not reach SQL");
}

/// One user must not be able to read or mutate another user's records.
#[tokio::test]
async fn users_cannot_reach_each_others_records() {
    // Holds the lock too: the projector claims work from the global outbox, so
    // an unlocked test can have its row processed by another test's drain.
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let owner = new_user();
    let intruder = new_user();

    let wallet = create_wallet(&app, owner, "usd").await;
    let purchase_id = create_purchase(&app, owner, wallet, "USD", 100).await;
    project(&pool).await;

    let (read, _) = get(&app, intruder, &format!("/purchase/{purchase_id}")).await;
    assert_eq!(read, StatusCode::NOT_FOUND, "reading another user's record");

    let (write, _) = put(
        &app,
        intruder,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": 1, "store": "Hijacked"}),
    )
    .await;
    assert_eq!(write, StatusCode::NOT_FOUND, "mutating another user's record");

    let (_, listed) = get(&app, intruder, "/purchase?limit=200").await;
    let ids = listed["data"].as_array().unwrap();
    assert!(
        !ids.iter().any(|p| p["id"] == purchase_id.to_string()),
        "another user's Purchase must not appear in my list"
    );

    // The owner still sees it, and its derived row is intact.
    assert_eq!(count_fact_rows(&pool, purchase_id).await, 1);
}

/// A field edit must queue exactly one projection event, and must queue one even
/// when no audit trail is supplied. The update path previously wrote two (and
/// skipped it entirely without a trail), so this guards that regression.
#[tokio::test]
async fn a_field_edit_enqueues_exactly_one_projection_event() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    let wallet = create_wallet(&app, user, "usd").await;
    let purchase_id = create_purchase(&app, user, wallet, "USD", 100).await;
    project(&pool).await;

    let before = outbox_rows_for_purchase(&pool, purchase_id).await;

    let (status, _) = put(
        &app,
        user,
        &format!("/purchase/{purchase_id}"),
        json!({"current_version": 1, "store": "Renamed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after = outbox_rows_for_purchase(&pool, purchase_id).await;
    assert_eq!(
        after - before,
        1,
        "one edit must queue exactly one projection event"
    );
}

/// The completion loop must finish what recording started: an unresolved
/// purchase gets its rate from the history/cache, the purchase updates through
/// the normal versioned path, and the projection rebuilds — marked as
/// system-resolved, so story 5's distinction is verifiable.
#[tokio::test]
async fn unresolved_rate_is_completed_by_the_worker() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    // Same precondition: clear the pair so the create records unresolved; the
    // worker then completes it from the cache row the test provides. Also clear
    // pending completion jobs: the shared queue holds jobs from earlier runs and
    // the worker claims the oldest first.
    execute(&pool, "DELETE FROM exchange_rate_cache WHERE from_currency='USD' AND to_currency='EUR'").await;
    execute(&pool, "DELETE FROM analytics_job WHERE kind='resolve_rate'").await;
    let wallet = create_wallet(&app, user, "eur").await;
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Later", "currency": "USD",
            "items": [{"name": "thing", "unit_price": 1000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(body["rate_source"], "unresolved");
    let purchase_id = body["id"].as_str().unwrap().parse().unwrap();
    project(&pool).await;

    // The unresolved purchase is listable (story 16).
    let conn = pool.get().await.unwrap();
    let list = conn
        .query(
            "SELECT purchase_id FROM purchase_fact WHERE resolution = 'unresolved' AND user_id = $1",
            &[&user],
        )
        .await
        .unwrap();
    assert_eq!(list.len(), 1, "the unresolved purchase is queryable");
    drop(conn);

    // A rate for the pair exists in the cache (written by any successful fetch);
    // the worker prefers it over calling the provider.
    let conn = pool.get().await.unwrap();
    conn.execute(
        r#"INSERT INTO exchange_rate_cache
               (from_currency, to_currency, rate_hour, rate_scaled, source)
           VALUES ('USD','EUR', date_trunc('hour', NOW()), 850000000, 'api')
           ON CONFLICT (from_currency, to_currency, rate_hour) DO UPDATE SET rate_scaled = 850000000"#,
        &[],
    )
    .await
    .unwrap();
    drop(conn);

    use financial_management::application::analytics::rate_completion::RateCompletionWorker;
    use financial_management::repository::{
        analytics::PgAnalyticsRepo, purchase::PgPurchaseRepo, rate_cache::PgRateCacheRepo,
        wallet::PgWalletRepo,
    };
    use financial_management::application::outbound::exchange_rate::MockExchangeRateProvider;

    let worker = RateCompletionWorker::new(
        PgAnalyticsRepo::new(pool.clone()),
        PgPurchaseRepo::new(pool.clone()),
        PgWalletRepo::new(pool.clone()),
        PgRateCacheRepo::new(pool.clone()),
        MockExchangeRateProvider::new(std::collections::HashMap::new()),
    );
    let handled = worker.run_once().await.expect("worker run");
    assert!(handled, "the resolve_rate job is claimed and handled");
    project(&pool).await;

    let (amount, rate, converted, resolution) = fact_row(&pool, purchase_id).await.expect("fact row");
    assert_eq!(amount, 1000);
    assert_eq!(rate, Some(850_000_000));
    assert_eq!(converted, Some(850));
    assert_eq!(resolution, "completed");

    // Story 5: the source is system-completed, not just 'api' or 'manual'.
    let (_, purchase) = get(&app, user, &format!("/purchase/{purchase_id}")).await;
    assert_eq!(purchase["rate_source"], "resolved");
    assert_eq!(purchase["exchange_rate_scaled"], 850_000_000);
}

/// The durable history must be a real fallback: after the unlogged cache is
/// emptied (which is what a restart does), completion must still succeed from
/// `fx_rate_history` without calling the provider.
#[tokio::test]
async fn completion_falls_back_to_the_durable_history() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    execute(&pool, "DELETE FROM exchange_rate_cache WHERE from_currency='USD' AND to_currency='EUR'").await;
    execute(&pool, "DELETE FROM fx_rate_history WHERE from_currency='USD' AND to_currency='EUR'").await;
    execute(&pool, "DELETE FROM analytics_job WHERE kind='resolve_rate'").await;

    let wallet = create_wallet(&app, user, "eur").await;
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Historic", "currency": "USD",
            "items": [{"name": "x", "unit_price": 2000, "quantity": 1}]
        }),
    )
    .await;
    assert_eq!(body["rate_source"], "unresolved");
    let purchase_id: uuid::Uuid = body["id"].as_str().unwrap().parse().unwrap();
    project(&pool).await;

    // Only the durable history knows this rate; the cache is deliberately empty
    // and the provider (mock with no pairs) would fail.
    execute(
        &pool,
        "INSERT INTO fx_rate_history (from_currency, to_currency, rate_hour, rate_scaled, source)
         VALUES ('USD','EUR', date_trunc('hour', NOW()), 900000000, 'api')",
    )
    .await;

    use financial_management::application::analytics::rate_completion::RateCompletionWorker;
    use financial_management::application::outbound::exchange_rate::MockExchangeRateProvider;
    use financial_management::repository::{
        analytics::PgAnalyticsRepo, purchase::PgPurchaseRepo, rate_cache::PgRateCacheRepo,
        wallet::PgWalletRepo,
    };

    let worker = RateCompletionWorker::new(
        PgAnalyticsRepo::new(pool.clone()),
        PgPurchaseRepo::new(pool.clone()),
        PgWalletRepo::new(pool.clone()),
        PgRateCacheRepo::new(pool.clone()),
        MockExchangeRateProvider::new(std::collections::HashMap::new()),
    );
    assert!(worker.run_once().await.expect("worker run"), "job claimed");
    project(&pool).await;

    let (_, rate, converted, resolution) = fact_row(&pool, purchase_id).await.expect("fact row");
    assert_eq!(rate, Some(900_000_000), "rate came from the history table");
    assert_eq!(converted, Some(1800), "2000 USD cents at 0.9");
    assert_eq!(resolution, "completed");
}

/// The sweep must not resurrect work that already exhausted its retries:
/// otherwise the retry bound means nothing.
#[tokio::test]
async fn the_sweep_does_not_requeue_exhausted_jobs() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let app = router(&pool);
    let user = new_user();

    execute(&pool, "DELETE FROM exchange_rate_cache WHERE from_currency='USD' AND to_currency='EUR'").await;
    execute(&pool, "DELETE FROM analytics_job WHERE kind='resolve_rate'").await;

    let wallet = create_wallet(&app, user, "eur").await;
    let (_, body) = post(
        &app,
        user,
        "/purchase/purchase_with_items",
        json!({
            "wallet_id": wallet, "store": "Exhausted", "currency": "USD",
            "items": [{"name": "x", "unit_price": 100, "quantity": 1}]
        }),
    )
    .await;
    let purchase_id = body["id"].as_str().unwrap();
    project(&pool).await;

    // Mark its completion job as exhausted.
    execute(
        &pool,
        &format!(
            "UPDATE analytics_job SET status='failed', attempts=10
              WHERE kind='resolve_rate' AND payload->>'purchase_id' = '{purchase_id}'"
        ),
    )
    .await;

    use financial_management::application::inbound::repositories::JobQueueRepository;
    let repo = financial_management::repository::analytics::PgAnalyticsRepo::new(pool.clone());
    // The sweep is operator-wide, so only this Purchase is asserted on: other
    // runs leave their own unresolved rows behind.
    repo.enqueue_resolve_jobs().await.expect("sweep");

    let conn = pool.get().await.unwrap();
    let n: i64 = conn
        .query_one(
            "SELECT count(*) AS n FROM analytics_job
              WHERE kind='resolve_rate' AND payload->>'purchase_id' = $1 AND status='pending'",
            &[&purchase_id.to_string()],
        )
        .await
        .unwrap()
        .get("n");
    assert_eq!(n, 0, "no new pending job was created");
}
