"""Tests for the classify handler.

These run against a real database, the way the Rust integration tests do,
because the handler's rules are database-shaped: the merchant's own history
is the predictor (D64), and the version on the purchase is the idempotency
key (D65).

The tests fail loudly without a database rather than skipping, per the
harness convention.
"""

from __future__ import annotations

import json
import os
import uuid

import psycopg
import pytest

from token_support import mint_service_token

from app.classify import (
    CONFIDENCE_THRESHOLD,
    MIN_MERCHANT_HISTORY,
    handle_classify,
)
from app.jobs import MAX_ATTEMPTS, claim_job, complete_job, fail_job, JobTokenError

OWNER_URL = os.environ.get("TEST_DATABASE_URL")
DATABASE_URL = os.environ.get("ANALYTICS_DB_URL")
pytestmark = pytest.mark.skipif(
    DATABASE_URL is None or OWNER_URL is None,
    reason="ANALYTICS_DB_URL or TEST_DATABASE_URL is not set",
)


@pytest.fixture()
def owner():
    """The database owner, which seeds fixtures and enqueues jobs.

    Enqueueing is the application's job (D4), so the tests do it as the owner
    the way the Rust app does. Reading fixtures back directly also needs the
    owner, because the sidecar's role is scoped by row-level security to one
    user at a time.
    """
    with psycopg.connect(OWNER_URL) as c:
        yield c


@pytest.fixture()
def conn(owner):
    """A connection AS the sidecar's role: the code under test's own view.

    Row-level security (U21) scopes it to whichever user it states, which is
    exactly what the handler relies on.
    """
    with psycopg.connect(DATABASE_URL) as c:
        yield c


@pytest.fixture(autouse=True)
def quiesce_pending_jobs(owner):
    """The job queue is global state, like the outbox: a pending job left by
    an earlier test would be claimed by this one, since the queue is served
    oldest-first. Fail or cancel everything pending before each test."""
    owner.execute(
        "UPDATE analytics_job SET status = 'failed', last_error = 'test quiesce' "
        " WHERE status IN ('pending', 'running')"
    )
    owner.commit()
    yield


@pytest.fixture()
def user():
    return str(uuid.uuid4())


def seed_category(owner, name: str, user_id: str | None = None) -> int:
    """A category, created as the owner the way the application does.

    There is no seed data: a rebuilt database has none, so depending on one
    would make the tests pass on a long-lived database and fail on a fresh
    one.
    """
    row = owner.execute(
        "INSERT INTO category (name, user_id, is_system) "
        "VALUES (%s, %s::uuid, %s) RETURNING id",
        (name, user_id, user_id is None),
    ).fetchone()
    owner.commit()
    return row[0]


def seed_merchant(owner, user_id: str, name: str) -> str:
    row = owner.execute(
        "INSERT INTO merchant (id, user_id, name, normalized_name) "
        "VALUES (gen_random_uuid(), %s::uuid, %s, lower(%s)) RETURNING id",
        (user_id, name, name),
    ).fetchone()
    owner.commit()
    return row[0]


def seed_purchase(
    owner, user_id: str, merchant_id: str, version: int = 1
) -> str:
    wallet = owner.execute(
        "INSERT INTO wallet (id, user_id, name, currency) "
        "VALUES (gen_random_uuid(), %s::uuid, %s, 'USD') RETURNING id",
        # Unique per call: two purchases at one merchant must not collide on
        # the (user_id, name) unique constraint.
        (user_id, f"wallet-{uuid.uuid4()}"),
    ).fetchone()[0]
    category = owner.execute(
        "SELECT id FROM category WHERE is_system ORDER BY id LIMIT 1"
    ).fetchone()
    category_id = category[0] if category else None
    purchase_id = str(uuid.uuid4())
    owner.execute(
        """
        INSERT INTO purchase (id, user_id, wallet_id, category_id, merchant_id,
                              store, purchase_date, currency, version,
                              classification_status)
        VALUES (%s::uuid, %s::uuid, %s, %s, %s, 'Blue Bottle', NOW(), 'USD',
                %s, 'unclassified')
        """,
        (purchase_id, user_id, wallet, category_id, merchant_id, version),
    )
    owner.commit()
    return purchase_id


def seed_label(owner, purchase_id: str, category_id: int) -> None:
    """Records that the user categorized this purchase, the way the
    application does: the label lives on the purchase itself (D63, the final
    settled value), which is what the handler's history query reads (D64)."""
    owner.execute(
        """
        UPDATE purchase
           SET category_id = %s, classification_status = 'confirmed'
         WHERE id = %s::uuid
        """,
        (category_id, purchase_id),
    )
    owner.commit()


def enqueue(
    owner,
    user_id: str,
    purchase_id: str,
    version: int | None,
    *,
    token_user: str | None = None,
    token: str | None = "mint",
) -> int:
    """Enqueues a classify job, with the service token the worker will verify.

    `token_user` mints the token for a different tenant than the row claims, and
    `token=None` omits it, so both refusals are testable.
    """
    from token_support import mint_service_token  # noqa: PLC0415 - import stays local

    if token == "mint":
        token = mint_service_token(token_user or user_id)

    row = owner.execute(
        """
        INSERT INTO analytics_job (user_id, kind, payload, auth_token)
        VALUES (%s::uuid, 'classify', %s, %s) RETURNING id
        """,
        (
            user_id,
            json.dumps({"purchase_id": purchase_id, "version": version}),
            token,
        ),
    ).fetchone()
    owner.commit()
    return row[0]


def test_a_merchant_with_a_consistent_history_suggests_that_category(owner, conn, user):
    """D64: the merchant's own history IS the cold start."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category_id = seed_category(owner, "Eating out", user)

    for _ in range(MIN_MERCHANT_HISTORY):
        purchase = seed_purchase(owner, user, merchant)
        seed_label(owner, purchase, category_id)

    # Purchase 41: the same merchant, no category yet.
    target = seed_purchase(owner, user, merchant)
    job_id = enqueue(owner, user, target, version=1)
    job = claim_job(conn)
    assert job is not None and job["id"] == job_id

    handle_classify(conn, job)
    complete_job(conn, job_id)
    conn.commit()

    row = owner.execute(
        "SELECT predicted_category_id, confidence, source "
        "FROM purchase_classification WHERE purchase_id = %s::uuid",
        (target,),
    ).fetchone()

    assert row is not None, "a prediction is recorded"
    assert row[0] == category_id, "the merchant's own category is suggested"
    assert row[2] == "rule", "it is a rule over the user's own data, not a model"
    assert row[1] >= CONFIDENCE_THRESHOLD


def test_one_purchase_is_an_anecdote_not_a_pattern(owner, conn, user):
    """D64's other half: MIN_MERCHANT_HISTORY exists because one example is
    not a pattern."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category_id = seed_category(owner, "Eating out", user)

    purchase = seed_purchase(owner, user, merchant)
    seed_label(owner, purchase, category_id)

    target = seed_purchase(owner, user, merchant)
    job = claim_job(conn)
    assert job is None, "no jobs enqueued yet"

    handle_classify(conn, {"id": 0, "user_id": user, "kind": "classify",
         "auth_token": mint_service_token(user),
                           "payload": {"purchase_id": target, "version": 1}})

    row = owner.execute(
        "SELECT predicted_category_id FROM purchase_classification "
        "WHERE purchase_id = %s::uuid",
        (target,),
    ).fetchone()
    assert row is None or row[0] is None, (
        "one purchase at a merchant is not enough to predict the next one"
    )


def test_a_divided_history_does_not_predict(owner, conn, user):
    """A merchant categorized two ways is not a pattern: the confidence falls
    with the disagreement, and the threshold (D47) refuses it."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category_a = seed_category(owner, "Drinks", user)
    category_b = seed_category(owner, "Eating out", user)
    categories = [category_a, category_b]

    for category_id in categories:
        purchase = seed_purchase(owner, user, merchant)
        seed_label(owner, purchase, category_id)

    target = seed_purchase(owner, user, merchant)
    handle_classify(
        conn,
        {"id": 0, "user_id": user, "kind": "classify",
         "auth_token": mint_service_token(user),
         "payload": {"purchase_id": target, "version": 1}},
    )

    row = owner.execute(
        "SELECT predicted_category_id, confidence FROM purchase_classification "
        "WHERE purchase_id = %s::uuid",
        (target,),
    ).fetchone()
    assert row is None or row[0] is None, (
        "a merchant categorized two ways must not predict either one"
    )


def test_a_purchase_the_user_categorized_is_never_touched(owner, conn, user):
    """D62: a user's choice is final. The handler fills empty categories
    only."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category_a = seed_category(owner, "Eating out", user)
    category_b = seed_category(owner, "Drinks", user)

    for category_id in (category_a, category_b):
        purchase = seed_purchase(owner, user, merchant)
        seed_label(owner, purchase, category_id)

    # The user picked B themselves. The history says A.
    target = seed_purchase(owner, user, merchant)
    owner.execute(
        "UPDATE purchase SET category_id = %s WHERE id = %s::uuid",
        (category_b, target),
    )
    owner.commit()

    handle_classify(
        conn,
        {"id": 0, "user_id": user, "kind": "classify",
         "auth_token": mint_service_token(user),
         "payload": {"purchase_id": target, "version": 1}},
    )

    row = owner.execute(
        "SELECT category_id FROM purchase WHERE id = %s::uuid", (target,)
    ).fetchone()
    assert row[0] == category_b, "the user's choice is untouched"


def test_a_stale_job_is_skipped(owner, conn, user):
    """D65: a job whose version is older than the purchase's is skipped, not
    run. The purchase moved on, and a stale suggestion would be wrong."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category_id = seed_category(owner, "Eating out", user)

    for _ in range(MIN_MERCHANT_HISTORY):
        purchase = seed_purchase(owner, user, merchant)
        seed_label(owner, purchase, category_id)

    target = seed_purchase(owner, user, merchant, version=2)
    job_id = enqueue(owner, user, target, version=1)
    job = claim_job(conn)

    handle_classify(conn, job)
    complete_job(conn, job_id)
    conn.commit()

    row = owner.execute(
        "SELECT predicted_category_id FROM purchase_classification "
        "WHERE purchase_id = %s::uuid",
        (target,),
    ).fetchone()
    assert row is None or row[0] is None, (
        "a stale job is skipped: the purchase moved on"
    )


def test_a_job_for_a_deleted_purchase_completes_without_error(owner, conn, user):
    """A purchase can be deleted after its job was enqueued. The job is not
    an error; there is simply nothing left to classify."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    purchase = seed_purchase(owner, user, merchant)
    job_id = enqueue(owner, user, purchase, version=1)

    owner.execute("DELETE FROM purchase WHERE id = %s::uuid", (purchase,))
    owner.commit()

    job = claim_job(conn)
    assert job is not None
    handle_classify(conn, job)  # must not raise
    complete_job(conn, job_id)
    conn.commit()


def test_an_unhandled_failure_is_recorded_and_schedules_a_retry(owner, conn, user):
    """The worker must survive a bad job: the attempt is recorded, the retry
    is scheduled, and the loop moves on."""
    job_id = enqueue(owner, user, "not-a-uuid", version=1)
    job = claim_job(conn)
    assert job is not None

    # A malformed id is a database error, not a ValueError: it surfaces as
    # whatever the query raises, and the worker records it either way.
    with pytest.raises(Exception):
        handle_classify(conn, job)

    fail_job(conn, job["id"], "classify job carries no purchase_id")
    conn.commit()

    row = owner.execute(
        "SELECT attempts, last_error, status FROM analytics_job WHERE id = %s",
        (job_id,),
    ).fetchone()
    assert row[0] == 1, "the attempt is recorded"
    assert row[1] is not None, "the error is recorded"
    assert row[2] == "pending", "a retry is scheduled"


def test_without_a_stated_user_the_worker_sees_nothing(owner, conn, user):
    """The RLS boundary holds under the worker's own path: a transaction that
    has not stated a user sees no rows at all, so a forgotten set_config is an
    empty result rather than every user's data."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category_id = seed_category(owner, "Eating out", user)
    purchase = seed_purchase(owner, user, merchant)
    seed_label(owner, purchase, category_id)

    row = conn.execute(
        "SELECT count(*) AS n FROM merchant WHERE user_id = %s::uuid",
        (user,),
    ).fetchone()

    assert row[0] == 0, "no user stated, nothing visible"


def test_a_prediction_never_overwrites_a_user_label(owner, conn, user):
    """Review finding 4: `record`'s upsert must not clobber a row the user
    made, which is what `classification_feedback` trains on."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    user_category = seed_category(owner, "Eating out", user)
    other_category = seed_category(owner, "Drinks", user)

    # Two purchases labeled 'Drinks' so the history would suggest 'Drinks'.
    for _ in range(MIN_MERCHANT_HISTORY):
        purchase = seed_purchase(owner, user, merchant)
        seed_label(owner, purchase, other_category)

    # The target has a USER label already, and a prediction row the user made
    # -- which is the row the upsert must not clobber.
    target = seed_purchase(owner, user, merchant)
    seed_label(owner, target, user_category)
    owner.execute(
        """
        INSERT INTO purchase_classification
            (purchase_id, predicted_category_id, confidence, model_version, source)
        VALUES (%s::uuid, %s, 1.0, 'user-v1', 'user')
        """,
        (target, user_category),
    )
    owner.commit()

    handle_classify(
        conn,
        {"id": 0, "user_id": user, "kind": "classify",
         "auth_token": mint_service_token(user),
         "payload": {"purchase_id": target, "version": 1}},
    )

    row = owner.execute(
        "SELECT predicted_category_id, source "
        "FROM purchase_classification WHERE purchase_id = %s::uuid",
        (target,),
    ).fetchone()
    assert row is not None and row[0] == user_category, "the user's label survives"
    assert row[1] == "user", "the row is still the user's, not the sidecar's"


def test_a_categorized_purchase_is_left_alone(owner, conn, user):
    """D62 plus 3.6: the worker does not touch a purchase the user categorized,
    and it does not overwrite the prediction the user was looking at when they
    chose. The disagreement is captured by the app when the correction is made,
    not re-invented here after the fact."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    other_category = seed_category(owner, "Drinks", user)

    # History says 'Drinks', consistently.
    for _ in range(MIN_MERCHANT_HISTORY):
        purchase = seed_purchase(owner, user, merchant)
        seed_label(owner, purchase, other_category)

    # The user picked something else for this one.
    target = seed_purchase(owner, user, merchant)
    user_choice = seed_category(owner, "Eating out", user)
    seed_label(owner, target, user_choice)

    # The prediction the user saw, recorded before they chose.
    owner.execute(
        """
        INSERT INTO purchase_classification
            (purchase_id, predicted_category_id, confidence, model_version, source,
             enqueued_version)
        VALUES (%s::uuid, %s, 0.9, 'merchant-history-v1', 'rule', 1)
        """,
        (target, other_category),
    )
    owner.commit()

    handle_classify(
        conn,
        {"id": 0, "user_id": user, "kind": "classify",
         "auth_token": mint_service_token(user),
         "payload": {"purchase_id": target, "version": 1}},
    )

    purchase_row = owner.execute(
        "SELECT category_id FROM purchase WHERE id = %s::uuid", (target,)
    ).fetchone()
    assert purchase_row[0] == user_choice, "the purchase is untouched"

    row = owner.execute(
        "SELECT predicted_category_id FROM purchase_classification WHERE purchase_id = %s::uuid",
        (target,),
    ).fetchone()
    assert row[0] == other_category, (
        "the prediction the user actually saw is preserved, not overwritten with "
        "a fresh one for a purchase that is no longer being classified"
    )


def test_retry_exhaustion_marks_the_job_failed(owner, conn, user):
    """Review finding 7: the fourth... the TENTH attempt must become `failed`
    rather than pending forever, and the delay doubles."""
    job_id = enqueue(owner, user, "not-a-uuid", version=1)

    for attempt in range(MAX_ATTEMPTS):
        job = claim_job(conn)
        assert job is not None, f"attempt {attempt + 1} is claimable"
        try:
            handle_classify(conn, job)
        except Exception:
            pass
        fail_job(conn, job["id"], "always fails")
        # The backoff is minutes; the test cannot wait for it, so the retry
        # is made due immediately -- what the test is checking is the bounds,
        # not the clock.
        owner.execute(
            "UPDATE analytics_job SET run_after = NOW() - INTERVAL '1 minute' "
            " WHERE id = %s",
            (job_id,),
        )
        owner.commit()

        row = owner.execute(
            "SELECT status, attempts, run_after FROM analytics_job WHERE id = %s",
            (job_id,),
        ).fetchone()
        assert row[0] == ("failed" if attempt + 1 >= MAX_ATTEMPTS else "pending"), (
            f"attempt {attempt + 1}: {row}"
        )

    # Nothing more is claimable.
    assert claim_job(conn) is None


def test_a_deleted_purchase_is_not_classified(owner, conn, user):
    """Review finding 11: a payload with no version would otherwise classify
    a soft-deleted purchase."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    purchase = seed_purchase(owner, user, merchant)

    owner.execute(
        "UPDATE purchase SET deleted_at = NOW() WHERE id = %s::uuid",
        (purchase,),
    )
    owner.commit()

    handle_classify(
        conn,
        {"id": 0, "user_id": user, "kind": "classify",
         "auth_token": mint_service_token(user),
         "payload": {"purchase_id": purchase, "version": None}},
    )

    row = owner.execute(
        "SELECT predicted_category_id FROM purchase_classification "
        "WHERE purchase_id = %s::uuid",
        (purchase,),
    ).fetchone()
    assert row is None, "a deleted purchase is not classified"


def test_a_job_without_a_token_is_refused(owner, conn, user):
    """D72: a job row names a tenant, but the row is readable by a compromised
    sidecar, so it must not be the thing that decides."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    purchase = seed_purchase(owner, user, merchant)
    enqueue(owner, user, purchase, version=1, token=None)

    job = claim_job(conn)
    assert job is not None
    with pytest.raises(JobTokenError):
        handle_classify(conn, job)


def test_a_job_whose_token_names_another_tenant_is_refused(owner, conn, user):
    """The row says one tenant, the credential says another: refuse, never pick."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    purchase = seed_purchase(owner, user, merchant)
    someone_else = str(uuid.uuid4())
    enqueue(owner, user, purchase, version=1, token_user=someone_else)

    job = claim_job(conn)
    assert job is not None
    with pytest.raises(JobTokenError):
        handle_classify(conn, job)

    # Nothing was recorded for either tenant.
    row = owner.execute(
        "SELECT 1 FROM purchase_classification WHERE purchase_id = %s::uuid",
        (purchase,),
    ).fetchone()
    assert row is None


def test_an_expired_job_token_is_refused(owner, conn, user):
    merchant = seed_merchant(owner, user, "Blue Bottle")
    purchase = seed_purchase(owner, user, merchant)
    enqueue(owner, user, purchase, version=1, token=mint_service_token(user, ttl=-60))

    job = claim_job(conn)
    assert job is not None
    with pytest.raises(JobTokenError):
        handle_classify(conn, job)


def test_a_valid_token_lets_the_job_run_for_its_tenant(owner, conn, user):
    """The positive case, so the suite cannot pass by refusing everything."""
    merchant = seed_merchant(owner, user, "Blue Bottle")
    category = seed_category(owner, "Eating out", user)
    for _ in range(MIN_MERCHANT_HISTORY):
        seed_label(owner, seed_purchase(owner, user, merchant), category)

    purchase = seed_purchase(owner, user, merchant)
    enqueue(owner, user, purchase, version=1)

    job = claim_job(conn)
    handle_classify(conn, job)

    row = owner.execute(
        "SELECT predicted_category_id FROM purchase_classification WHERE purchase_id = %s::uuid",
        (purchase,),
    ).fetchone()
    assert row is not None and row[0] == category
