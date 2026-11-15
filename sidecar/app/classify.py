"""Suggesting a category for a purchase.

The rules the grilling settled (D62-D67) are all here:

  - A purchase the user categorized is NEVER changed (D62). The handler only
    fills a category that is still NULL.
  - The cold start is the merchant's own history (D64): if every purchase the
    user recorded at a merchant carried one category, that category is
    suggested for the next one, at a confidence that reflects how consistent
    the history is. No model needed.
  - The purchase's version is the idempotency key (D65): a job whose version
    is older than the purchase's current version is skipped, because the
    purchase has moved on and a stale suggestion would be wrong.
  - This handler returns the suggestion; it does NOT write to `purchase`.
    The application owns that write (D66). What it does write is a
    `purchase_classification` row, which is analytics-owned and safe.
"""

from __future__ import annotations

import json
import logging

import psycopg

log = logging.getLogger(__name__)

# Below this, no suggestion is made: the purchase stays uncategorized and the
# user is asked (D47).
CONFIDENCE_THRESHOLD = 0.75

# How many purchases of one merchant are enough to make its history a
# prediction rather than an anecdote.
MIN_MERCHANT_HISTORY = 2


def handle_classify(conn: psycopg.Connection, job: dict) -> None:
    """Suggests a category for the job's purchase, and records the prediction.

    The tenant is stated HERE, in the transaction the work happens in. The
    scope spans exactly this transaction: row-level security (U21) filters
    every query to this user, not the WHERE clauses the code remembers.
    """
    payload = job["payload"]
    purchase_id = payload.get("purchase_id")
    version = payload.get("version")

    if purchase_id is None:
        raise ValueError("classify job carries no purchase_id")

    # D72: the tenant comes from the verified claim, not from the row. A
    # compromised sidecar can read `analytics_job`, so the column that names the
    # tenant must not be the thing that decides it -- it is only compared
    # against the claim, and a mismatch is refused.
    from app.jobs import JobTokenError
    from app.service_auth import ServiceTokenError, verify_for_tenant

    try:
        identity = verify_for_tenant(job.get("auth_token"), job["user_id"])
    except ServiceTokenError as exc:
        raise JobTokenError(str(exc)) from exc

    with conn.transaction():
        conn.execute(
            "SELECT set_config('app.user_id', %s, true)",
            (identity.user_id,),
        )
        _classify(conn, str(purchase_id), version)


def _classify(conn: psycopg.Connection, purchase_id: str, version) -> None:
    """The work, inside the tenant's scope."""
    row = conn.execute(
        """
        SELECT p.version, p.category_id, p.merchant_id, p.store,
               p.description, p.purchase_date
          FROM purchase p
         WHERE p.id = %s::uuid
           AND p.deleted_at IS NULL
        """,
        (purchase_id,),
    ).fetchone()

    if row is None:
        # The purchase was deleted after the job was enqueued.
        log.info("purchase %s is gone; nothing to classify", purchase_id)
        return

    purchase_version, category_id, merchant_id, store, description, purchase_date = row

    # D65: a stale job is skipped, not run. The purchase has moved on since
    # this job was enqueued, and a suggestion computed from an older state
    # would be wrong.
    if version is not None and int(version) != purchase_version:
        log.info(
            "purchase %s moved from version %s to %s; skipping",
            purchase_id, version, purchase_version,
        )
        return

    # D62: a purchase the user categorized is final -- but the system's
    # disagreement is still data, so it is recorded for the learning loop
    # without touching the purchase.
    if category_id is not None:
        disagreement = suggest(conn, purchase_id, merchant_id)
        if disagreement is not None:
            record(
                conn,
                purchase_id,
                disagreement["category_id"],
                disagreement["confidence"],
                "rule",
                purchase_version,
                json.dumps(disagreement),
            )
            log.info(
                "purchase %s is categorized %s; the system would have said %s, recorded but not applied",
                purchase_id, category_id, disagreement["category_id"],
            )
        return

    suggestion = suggest(conn, purchase_id, merchant_id)

    if suggestion is None:
        # The purchase stays uncategorized: saying "I do not know" is the
        # honest answer, and the user is asked (D47).
        record(conn, purchase_id, None, 0.0, "rule", purchase_version, None)
        return

    record(
        conn,
        purchase_id,
        suggestion["category_id"],
        suggestion["confidence"],
        "rule",
        purchase_version,
        json.dumps(suggestion),
    )
    log.info(
        "purchase %s suggested category %s at %.2f",
        purchase_id, suggestion["category_id"], suggestion["confidence"],
    )


def suggest(
    conn: psycopg.Connection,
    purchase_id: str,
    merchant_id: int | None,
) -> dict | None:
    """Suggests a category from the merchant's own history.

    With no model and no labels beyond the user's own categorizations, the
    merchant's consistency IS the predictor (D64): a merchant the user has
    always categorized the same way is one the next purchase there can be
    suggested for.
    """
    if merchant_id is None:
        # Nothing to be consistent about: the merchant is unknown, so the
        # honest answer is that we do not know.
        return None

    row = conn.execute(
        """
        WITH history AS (
            SELECT p.category_id, COUNT(*) AS purchases
              FROM purchase p
             WHERE p.merchant_id = %s
               AND p.deleted_at IS NULL
               AND p.category_id IS NOT NULL
               -- Exclude the purchase being classified: its own earlier
               -- prediction must not be counted as evidence (review
               -- finding 12).
               AND p.id <> %s::uuid
             GROUP BY p.category_id
        )
        SELECT h.category_id,
               h.purchases,
               (SELECT SUM(purchases) FROM history) AS total,
               (SELECT COUNT(*) FROM history) AS distinct_categories
          FROM history h
         ORDER BY h.purchases DESC
         LIMIT 1
        """,
        (merchant_id, purchase_id),
    ).fetchone()

    if row is None:
        return None

    category_id, purchases, total, distinct_categories = row

    # One purchase is an anecdote, not a pattern (MIN_MERCHANT_HISTORY). And
    # a merchant the user has categorized two different ways is not a pattern
    # either: the confidence falls with the disagreement.
    if purchases < MIN_MERCHANT_HISTORY or distinct_categories > 1:
        return None

    confidence = purchases / total if total else 0.0
    if confidence < CONFIDENCE_THRESHOLD:
        return None

    return {
        "category_id": int(category_id),
        "confidence": float(confidence),
        "merchant_id": int(merchant_id),
        "basis": "merchant_history",
    }


def record(
    conn: psycopg.Connection,
    purchase_id: str,
    category_id: int | None,
    confidence: float,
    source: str,
    version: int,
    result_json: str | None,
) -> None:
    """Records the prediction the loop made.

    Upsert keyed on the purchase: a purchase is classified once, so running
    the same job twice writes the same row twice and is harmless (D65's other
    half).
    """
    # An existing row the user made is not overwritten: a prediction must not
    # clobber a label, which is what `classification_feedback` trains on
    # (review finding 4).
    conn.execute(
        """
        INSERT INTO purchase_classification
            (purchase_id, predicted_category_id, confidence, model_version, source,
             enqueued_version)
        VALUES (%s::uuid, %s, %s, %s, %s, %s)
        ON CONFLICT (purchase_id) DO NOTHING
        """,
        (
            purchase_id,
            category_id,
            confidence,
            "merchant-history-v1" if source == "rule" else source,
            source,
            # D65: the version this suggestion was computed at, so the applier
            # can refuse it if the purchase has moved on since.
            version,
        ),
    )


def suggest_for_purchase(purchase_id: str, user_id: str) -> dict | None:
    """The suggestion an inline call asks for, read-only (D66, D67).

    This is the same predictor the worker uses, without the writes: the sidecar
    never touches `purchase`, so the caller applies the suggestion. Returns
    `None` when there is nothing honest to say, which is an answer, not a
    failure.

    A database that is not configured, or unreachable, raises: the caller must
    be able to tell "no suggestion" from "I could not look".
    """
    from app import db

    url = db.database_url()
    if not url:
        raise RuntimeError(
            "ANALYTICS_DB_URL is not set, so no suggestion can be computed"
        )

    with db.pool().connection() as conn:
        with conn.transaction():
            # The tenant is stated in the transaction the work happens in, and
            # it comes from the verified claim (the caller passed it), not from
            # the request body.
            conn.execute("SELECT set_config('app.user_id', %s, true)", (user_id,))
            row = conn.execute(
                "SELECT merchant_id FROM purchase WHERE id = %s::uuid AND deleted_at IS NULL",
                (purchase_id,),
            ).fetchone()
            if row is None:
                return None
            return suggest(conn, purchase_id, row[0])
