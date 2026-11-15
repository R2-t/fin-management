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
