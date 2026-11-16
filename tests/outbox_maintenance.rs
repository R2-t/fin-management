//! Integration tests for outbox maintenance: purging processed work and
//! surfacing work that has exhausted its retries.
//!
//! Seam: the maintenance operations' public interface, which the CLI commands
//! wrap. Arrangement inserts rows directly (fixture), assertions use the
//! interface where one exists.

mod common;

use financial_management::application::inbound::repositories::AnalyticsRepository;
use common::*;
use financial_management::application::inbound::repositories::MAX_ATTEMPTS;
use financial_management::application::analytics::maintenance::{
    purge_processed_outbox, stuck_outbox_report,
};

/// Processed work older than the retention window is removed; pending work is
/// never removed, because deleting it would lose a Purchase's projection.
#[tokio::test]
async fn purge_removes_only_processed_work_older_than_the_window() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let repo = facts(&pool);
    let user = new_user();

    // Arrange: one long-processed row (eligible), one recently processed row
    // (too new), and one still pending (must survive).
    execute(
        &pool,
        &format!(
            "INSERT INTO analytics_outbox
                 (user_id, aggregate_type, aggregate_id, aggregate_version, created_at, processed_at)
             VALUES
                 ('{user}', 'purchase', gen_random_uuid(), 1, NOW() - INTERVAL '40 days', NOW() - INTERVAL '30 days'),
                 ('{user}', 'purchase', gen_random_uuid(), 1, NOW() - INTERVAL '2 days',  NOW() - INTERVAL '1 day'),
                 ('{user}', 'purchase', gen_random_uuid(), 1, NOW(),                    NULL)"
        ),
    )
    .await;

    let removed = purge_processed_outbox(&repo, std::time::Duration::from_secs(7 * 24 * 3600))
        .await
        .expect("purge failed");

    assert_eq!(removed, 1, "only the row past the retention window may be purged");

    assert_eq!(
        pending_outbox_for_user(&pool, user).await,
        1,
        "the pending row must survive the purge"
    );

    // A second purge finds nothing new to remove.
    let again = purge_processed_outbox(&repo, std::time::Duration::from_secs(7 * 24 * 3600))
        .await
        .expect("purge failed");
    assert_eq!(again, 0);
}

/// Work that exhausts its retries must be reportable rather than silently
/// abandoned; work that is merely waiting must not be reported as stuck.
#[tokio::test]
async fn exhausted_rows_are_reported_and_healthy_rows_are_not() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let repo = facts(&pool);
    let user = new_user();

    execute(
        &pool,
        &format!(
            "INSERT INTO analytics_outbox
                 (user_id, aggregate_type, aggregate_id, aggregate_version, attempts, last_error)
             VALUES
                 ('{user}', 'purchase', gen_random_uuid(), 1, {cap}, 'projection exploded'),
                 ('{user}', 'purchase', gen_random_uuid(), 1, 1, NULL)",
            cap = MAX_ATTEMPTS
        ),
    )
    .await;

    // The report is deliberately global (it is an operator view), so assert on
    // this test's own rows rather than the total.
    let report: Vec<_> = stuck_outbox_report(&repo)
        .await
        .expect("report failed")
        .into_iter()
        .filter(|s| s.user_id == user)
        .collect();

    assert_eq!(report.len(), 1, "only the exhausted row is stuck");
    assert_eq!(report[0].attempts, MAX_ATTEMPTS);
    assert_eq!(report[0].last_error.as_deref(), Some("projection exploded"));
}

/// A permanently stuck row must not masquerade as ever-growing projection lag,
/// or the lag reading becomes a floor that never clears.
#[tokio::test]
async fn lag_ignores_exhausted_rows() {
    let _guard = projection_lock().await;
    let pool = test_pool().await;
    let repo = facts(&pool);
    let user = new_user();

    // The lag reading is an operator metric over the whole outbox, and other
    // tests leave pending fixture rows behind. Quiesce those first so the
    // reading reflects only the rows this test controls.
    execute(
        &pool,
        "UPDATE analytics_outbox SET processed_at = NOW() WHERE processed_at IS NULL",
    )
    .await;

    execute(
        &pool,
        &format!(
            "INSERT INTO analytics_outbox
                 (user_id, aggregate_type, aggregate_id, aggregate_version, created_at, attempts)
             VALUES
                 ('{user}', 'purchase', gen_random_uuid(), 1, NOW() - INTERVAL '1 hour', {cap}),
                 ('{user}', 'purchase', gen_random_uuid(), 1, NOW(), 0)",
            cap = MAX_ATTEMPTS
        ),
    )
    .await;

    let lag = repo
        .outbox_lag_seconds()
        .await
        .expect("lag query failed")
        .expect("the fresh pending row is lag");
    assert!(
        lag < 60.0,
        "an hour-old exhausted row must not inflate the lag reading; got {lag}s"
    );
}
