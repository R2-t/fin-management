//! The producer side of D72: a purchase the user did not categorize becomes a
//! `classify` job that carries a credential naming its tenant.
//!
//! The worker's half is tested in `sidecar/tests/test_classify.py`; these tests
//! are the other end of the same contract.

mod common;

use common::test_pool;
use financial_management::repository::analytics_job::enqueue_classify;

fn token() -> String {
    // Shaped like a minted PASETO, but the enqueue only stores it: the worker is
    // what verifies it, and it is not running here.
    "v4.local.fake-but-shaped-like-a-token".to_string()
}

#[tokio::test]
async fn an_uncategorized_purchase_gets_a_job_carrying_a_credential() {
    let pool = test_pool().await;
    let user = common::new_user();
    let purchase = uuid::Uuid::new_v4();

    let inserted = enqueue_classify(&pool, user, purchase, 1, &token())
        .await
        .expect("the job is queued");
    assert!(inserted);

    let conn = pool.get().await.unwrap();
    let row = conn
        .query_one(
            "SELECT kind, payload->>'purchase_id' AS purchase_id, auth_token, status
               FROM analytics_job
              WHERE user_id = $1 AND kind = 'classify'",
            &[&user],
        )
        .await
        .unwrap();

    assert_eq!(row.get::<_, String>("kind"), "classify");
    assert_eq!(
        row.get::<_, String>("purchase_id"),
        purchase.to_string(),
        "the job names the purchase it is about"
    );
    assert_eq!(
        row.get::<_, String>("auth_token"),
        token(),
        "the row carries the credential that decides the tenant (D72)"
    );
    assert_eq!(row.get::<_, String>("status"), "pending");
}

#[tokio::test]
async fn a_second_enqueue_for_the_same_purchase_is_a_no_op() {
    let pool = test_pool().await;
    let user = common::new_user();
    let purchase = uuid::Uuid::new_v4();

    assert!(enqueue_classify(&pool, user, purchase, 1, &token()).await.unwrap());
    assert!(
        !enqueue_classify(&pool, user, purchase, 1, &token()).await.unwrap(),
        "a retried create must not double the work"
    );

    let conn = pool.get().await.unwrap();
    let count: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM analytics_job WHERE user_id = $1 AND kind = 'classify'",
            &[&user],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
}

#[tokio::test]
async fn a_job_that_already_ran_does_not_block_a_new_one() {
    let pool = test_pool().await;
    let user = common::new_user();
    let purchase = uuid::Uuid::new_v4();

    enqueue_classify(&pool, user, purchase, 1, &token()).await.unwrap();
    {
        let conn = pool.get().await.unwrap();
        conn.execute(
            "UPDATE analytics_job SET status = 'done' WHERE user_id = $1 AND kind = 'classify'",
            &[&user],
        )
        .await
        .unwrap();
    }

    assert!(
        enqueue_classify(&pool, user, purchase, 2, &token()).await.unwrap(),
        "an edited purchase is classified again"
    );
}

/// The queued path was a dead end: the worker records a suggestion and nothing
/// applied it. This is the consumer, tested end to end against the database.
#[tokio::test]
async fn a_recorded_suggestion_is_applied_to_the_purchase() {
    let pool = test_pool().await;
    let user = common::new_user();
    let app = common::router(&pool);
    let wallet = common::create_wallet(&app, user, "USD").await;
    let category = create_category(&pool, user).await;

    let _lock = common::projection_lock().await;
    let purchase = common::create_purchase(&app, user, wallet, "USD", 1000).await;

    // What the worker does: record the prediction, do not touch `purchase`.
    {
        let conn = pool.get().await.unwrap();
        conn.execute(
            "INSERT INTO purchase_classification
                 (purchase_id, predicted_category_id, confidence, model_version, source)
             VALUES ($1, $2, 0.9, 'merchant-history-v1', 'rule')",
            &[&purchase, &category],
        )
        .await
        .unwrap();
    }

    // The repository write path, called directly so the test does not depend on
    // whatever else is in the shared queue.
    let applied = financial_management::repository::purchase::apply_suggested_category(
        &pool,
        user,
        purchase,
        category,
        0.9,
        "merchant-history-v1",
    )
    .await
    .expect("the suggestion is applied");
    assert!(applied, "a NULL category is filled");

    let conn = pool.get().await.unwrap();
    let row = conn
        .query_one(
            "SELECT category_id, classification_status FROM purchase WHERE id = $1",
            &[&purchase],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<i32>>("category_id"), Some(category));
    assert_eq!(row.get::<_, String>("classification_status"), "suggested");

    // And it went through the write path: a version bump and an outbox row, so
    // the projection rebuilds instead of staying stale.
    let outbox: i64 = conn
        .query_one(
            "SELECT COUNT(*) FROM analytics_outbox WHERE aggregate_id = $1 AND user_id = $2",
            &[&purchase, &user],
        )
        .await
        .unwrap()
        .get(0);
    assert!(outbox >= 1, "the outbox was told to rebuild");
}

/// The user's choice always wins (D62).
#[tokio::test]
async fn a_suggestion_never_overwrites_a_category_the_user_chose() {
    let pool = test_pool().await;
    let user = common::new_user();
    let app = common::router(&pool);
    let wallet = common::create_wallet(&app, user, "USD").await;
    let suggested = create_category(&pool, user).await;
    let chosen = create_category(&pool, user).await;

    let _lock = common::projection_lock().await;
    let purchase = common::create_purchase(&app, user, wallet, "USD", 1000).await;
    {
        let conn = pool.get().await.unwrap();
        conn.execute(
            "UPDATE purchase SET category_id = $1, classification_status = 'confirmed'
              WHERE id = $2",
            &[&chosen, &purchase],
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO purchase_classification
                 (purchase_id, predicted_category_id, confidence, model_version, source)
             VALUES ($1, $2, 0.9, 'merchant-history-v1', 'rule')",
            &[&purchase, &suggested],
        )
        .await
        .unwrap();
    }

    financial_management::repository::purchase::apply_suggested_category(
        &pool,
        user,
        purchase,
        suggested,
        0.9,
        "merchant-history-v1",
    )
    .await
    .unwrap();

    let conn = pool.get().await.unwrap();
    let row = conn
        .query_one("SELECT category_id FROM purchase WHERE id = $1", &[&purchase])
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, Option<i32>>("category_id"),
        Some(chosen),
        "the user's own category is never replaced by a suggestion"
    );
}

async fn create_category(pool: &financial_management::repository::PgPool, user: uuid::Uuid) -> i32 {
    let conn = pool.get().await.unwrap();
    conn.query_one(
        "INSERT INTO category (user_id, name) VALUES ($1, $2) RETURNING id",
        &[&user, &format!("cat-{}", uuid::Uuid::new_v4())],
    )
    .await
    .unwrap()
    .get(0)
}

/// The production loop: everything the worker recorded is applied in one pass.
#[tokio::test]
async fn the_batch_consumer_applies_everything_pending() {
    let pool = test_pool().await;
    let user = common::new_user();
    let app = common::router(&pool);
    let wallet = common::create_wallet(&app, user, "USD").await;
    let category = create_category(&pool, user).await;

    let _lock = common::projection_lock().await;
    let purchase = common::create_purchase(&app, user, wallet, "USD", 1000).await;
    {
        let conn = pool.get().await.unwrap();
        conn.execute(
            "INSERT INTO purchase_classification
                 (purchase_id, predicted_category_id, confidence, model_version, source)
             VALUES ($1, $2, 0.9, 'merchant-history-v1', 'rule')
             ON CONFLICT (purchase_id) DO NOTHING",
            &[&purchase, &category],
        )
        .await
        .unwrap();
    }

    // Drain in a loop: the queue is shared with other tests, so one pass may
    // spend its limit elsewhere.
    for _ in 0..5 {
        financial_management::apply_pending_suggestions(&pool, 100)
            .await
            .expect("the consumer runs");
    }

    let conn = pool.get().await.unwrap();
    let row = conn
        .query_one("SELECT category_id FROM purchase WHERE id = $1", &[&purchase])
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<i32>>("category_id"), Some(category));
}
