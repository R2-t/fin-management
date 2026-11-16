//! Integration tests for the sidecar's row-level security.
//!
//! The `analytics` role can connect to the database. What it may see is a
//! property of the database now (U21), not of the queries it happens to send:
//! without stating which user it acts for, it sees nothing, and it can never
//! see another user's rows.
//!
//! These tests connect AS the sidecar's role. Everything else in the suite
//! connects as the owner, which bypasses row-level security.

mod common;

use financial_management::repository::PgPool;

/// A connection as the sidecar's role, not as the owner.
///
/// The test database URL carries the owner's credentials; the sidecar's are
/// substituted in. `analytics` is the role U18 created, with the development
/// password U18 sets.
async fn sidecar_pool() -> financial_management::repository::PgPool {
    let owner = common::test_dsn();
    let sidecar = owner.replacen(
        "postgres:postgres@",
        "analytics:analytics-dev-only@",
        1,
    );
    financial_management::connect_pool(&sidecar)
        .await
        .expect("could not connect as the analytics role")
}

/// A merchant the test can see or not see, depending on who is asking.
async fn seed_merchant(
    owner_pool: &PgPool,
    user_id: uuid::Uuid,
    name: &str,
) -> uuid::Uuid {
    let conn = owner_pool.get().await.expect("pool checkout");
    let row = conn
        .query_one(
            "INSERT INTO merchant (id, user_id, name, normalized_name)
             VALUES (gen_random_uuid(), $1, $2, lower($2))
             RETURNING id",
            &[&user_id, &name],
        )
        .await
        .expect("seed merchant");
    row.get("id")
}

/// Without stating a user, the sidecar's role sees nothing -- so a forgotten
/// `set_config` is an empty result, not every user's data.
#[tokio::test]
async fn without_a_stated_user_the_sidecar_sees_nothing() {
    let owner_pool = common::test_pool().await;
    let user = uuid::Uuid::new_v4();
    seed_merchant(&owner_pool, user, "NETFLIX").await;

    let pool = sidecar_pool().await;
    let conn = pool.get().await.expect("pool checkout");

    let row = conn
        .query_one(
            "SELECT count(*) AS n FROM merchant WHERE user_id = $1",
            &[&user],
        )
        .await
        .expect("query");
    let n: i64 = row.get("n");

    assert_eq!(
        n, 0,
        "a connection that has not stated a user must see no rows at all"
    );
}

/// A stated user that is not the row's owner sees nothing: the boundary is
/// the database's, not the query's.
#[tokio::test]
async fn another_users_merchant_is_invisible() {
    let owner_pool = common::test_pool().await;
    let owner = uuid::Uuid::new_v4();
    let intruder = uuid::Uuid::new_v4();
    let merchant_id = seed_merchant(&owner_pool, owner, "NETFLIX").await;

    let pool = sidecar_pool().await;
    let conn = pool.get().await.expect("pool checkout");
    conn.execute(
        "SELECT set_config('app.user_id', $1, false)",
        &[&intruder.to_string()],
    )
    .await
    .expect("set app.user_id");

    let row = conn
        .query_one(
            "SELECT count(*) AS n FROM merchant WHERE id = $1",
            &[&merchant_id],
        )
        .await
        .expect("query");
    let n: i64 = row.get("n");

    assert_eq!(n, 0, "another user's merchant must be invisible");
}

/// The right user sees their own rows, and their aliases through them.
#[tokio::test]
async fn the_owning_user_sees_their_rows() {
    let owner_pool = common::test_pool().await;
    let user = uuid::Uuid::new_v4();
    let merchant_id = seed_merchant(&owner_pool, user, "NETFLIX").await;

    {
        let conn = owner_pool.get().await.expect("pool checkout");
        conn.execute(
            "INSERT INTO merchant_alias (id, merchant_id, alias, normalized_alias)
             VALUES (gen_random_uuid(), $1, 'netflix.com', 'netflix.com')",
            &[&merchant_id],
        )
        .await
        .expect("seed alias");
    }

    let pool = sidecar_pool().await;
    let conn = pool.get().await.expect("pool checkout");
    conn.execute(
        "SELECT set_config('app.user_id', $1, false)",
        &[&user.to_string()],
    )
    .await
    .expect("set app.user_id");

    let row = conn
        .query_one("SELECT count(*) AS n FROM merchant WHERE user_id = $1", &[&user])
        .await
        .expect("query");
    assert_eq!(row.get::<_, i64>("n"), 1, "their own merchant is visible");

    let row = conn
        .query_one("SELECT count(*) AS n FROM merchant_alias", &[])
        .await
        .expect("query");
    assert_eq!(
        row.get::<_, i64>("n"),
        1,
        "the alias is visible through its merchant, without its own WHERE clause"
    );
}

/// The owner bypasses row-level security, which is what keeps the application
/// -- which scopes its own queries -- unaffected.
#[tokio::test]
async fn the_owner_is_not_scoped() {
    let owner_pool = common::test_pool().await;
    let user = uuid::Uuid::new_v4();
    seed_merchant(&owner_pool, user, "NETFLIX").await;

    // The owner sees the row with no app.user_id set at all.
    let conn = owner_pool.get().await.expect("pool checkout");
    let row = conn
        .query_one(
            "SELECT count(*) AS n FROM merchant WHERE user_id = $1",
            &[&user],
        )
        .await
        .expect("query");

    assert_eq!(
        row.get::<_, i64>("n"),
        1,
        "the owner's own scoping is in its queries; it must not be scoped twice"
    );
}

/// U33: the training job's role reads across tenants, and only what labels are
/// built from.
///
/// It exists because training is cross-user and the `analytics` role is
/// RLS-scoped to one tenant. The alternative -- the owner connection -- handed
/// the sidecar full DML on every tenant, including password hashes.
#[tokio::test]
async fn the_trainer_role_reads_labels_and_nothing_else() {
    let dsn = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let trainer_dsn = dsn.replace("postgres:postgres@", "trainer:trainer-dev-only@");
    let pool = financial_management::connect_pool(&trainer_dsn)
        .await
        .expect("the trainer role can connect");
    let conn = pool.get().await.expect("checkout");

    // It must be able to read the label source across tenants.
    for table in ["purchase", "category", "merchant"] {
        conn.query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .unwrap_or_else(|e| panic!("trainer must read {table}: {e}"));
    }

    // And it must not reach the money, the identity, or the ledger.
    for table in ["users", "sessions", "wallet", "ledger_entry"] {
        let result = conn.query_one(&format!("SELECT count(*) FROM {table}"), &[]).await;
        assert!(result.is_err(), "trainer must NOT read {table}");
    }

    // Read-only on the label source: the sidecar never writes a purchase (D4).
    let write = conn.execute("UPDATE purchase SET store = store", &[]).await;
    assert!(write.is_err(), "trainer must NOT write purchase");
}
