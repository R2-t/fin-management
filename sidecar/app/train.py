"""The training job (3.7): temporal split, and D48's promotion gate.

Run it with `python -m app.train`.

Two rules decide everything here:

  - The split is temporal, never random. A random split lets the model see next
    month's purchases while being tested on this month's, which reports an
    accuracy it will never have in use.
  - A candidate is promoted only if it beats the active model on **every** metric
    (D48). "Better on average" is how a model that got worse at the category you
    care about gets shipped.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import sys

import psycopg

from app import db, model

log = logging.getLogger(__name__)

# The oldest 80% trains, the newest 20% tests. The test set is the future, which
# is the only honest thing to test a forecast-like model on.
TRAIN_FRACTION = 0.8

METRICS = ("accuracy", "coverage")

# Below this many held-out rows the comparison is noise: one answer moves
# accuracy by 20 points on a five-row exam. The merchant-history rule already
# covers the cold start, so waiting costs nothing (D82).
MIN_HELD_OUT = 30

# The owner connection the training job needs, because the model is cross-user
# and the sidecar's own role is RLS-scoped to one tenant.
TRAIN_DB_URL_ENV = "ANALYTICS_TRAIN_DB_URL"


def labelled_purchases(conn: psycopg.Connection) -> list[tuple[str, str, object]]:
    """Every user's categorized purchases, oldest first.

    Cross-user on purpose: a merchant means the same thing to everybody, and one
    person's few hundred labelled purchases will never teach a model what
    "SHELL" is. What is *not* shared is the category itself -- category rows
    belong to a user, so the label is the category's **name**, normalised. The
    prediction is a name, and the caller maps it to a category that user owns.

    The label is `purchase.category_id` -- the value the user finally left in
    place (D63) -- and never a suggestion the application applied on the model's
    own advice, which would make it learn to reproduce itself.
    """
    rows = conn.execute(
        """
        SELECT p.id,
               lower(btrim(c.name)) AS label,
               COALESCE(p.store, '') || ' ' || COALESCE(p.description, '') AS text,
               p.purchase_date
          FROM purchase p
          JOIN category c ON c.id = p.category_id
         WHERE p.category_id IS NOT NULL
           AND p.deleted_at IS NULL
           AND c.name IS NOT NULL
           AND btrim(c.name) <> ''
         ORDER BY p.purchase_date, p.created_at
        """
    ).fetchall()
    return [(row[2], row[1], row[3]) for row in rows]


def active_model(conn: psycopg.Connection) -> dict | None:
    """The active model, with the artifact so it can be scored again.

    Its *stored* metrics are not used for comparison: they were measured on the
    held-out rows of its own training run, which are not the rows the candidate
    is measured on (D82).
    """
    row = conn.execute(
        "SELECT model_version, metrics, artifact FROM model_registry "
        " WHERE active AND artifact IS NOT NULL"
    ).fetchone()
    if row is None:
        return None
    metrics = row[1] if isinstance(row[1], dict) else json.loads(row[1])
    artifact = row[2] if isinstance(row[2], dict) else json.loads(row[2])
    return {"model_version": row[0], "metrics": metrics, "artifact": artifact}


def beats(candidate: dict, incumbent: dict | None) -> bool:
    """D48: every metric must be at least as good, and one strictly better.

    With no incumbent the candidate is promoted, which is how the first model
    gets in -- after it has already been measured on held-out data.
    """
    if incumbent is None:
        return True
    for metric in METRICS:
        c = candidate.get(metric, 0.0)
        i = incumbent.get(metric, 0.0)
        if c < i:
            return False
    return any(candidate.get(m, 0.0) > incumbent.get(m, 0.0) for m in METRICS)


def run(conn: psycopg.Connection) -> dict | None:
    """Trains, evaluates, and registers. Returns the registered row, if any."""
    labelled = labelled_purchases(conn)
    split = int(len(labelled) * TRAIN_FRACTION)
    training = [(text, category) for text, category, _ in labelled[:split]]
    held_out = [(text, category) for text, category, _ in labelled[split:]]

    try:
        artifact = model.train(training)
    except model.NotEnoughData as exc:
        log.warning("not training: %s", exc)
        return None

    metrics = model.evaluate(artifact, held_out)
    incumbent = active_model(conn)

    # Both models answer the SAME rows (D82). Comparing the candidate on today's
    # window with the incumbent's stored numbers compares two different exams.
    if incumbent is not None:
        incumbent["metrics"] = model.evaluate(incumbent["artifact"], held_out)

    # Below the minimum the numbers cannot separate two models, so the gate
    # refuses to speak -- and says why, because a silent refusal looks like a
    # broken job (D82).
    if len(held_out) < MIN_HELD_OUT:
        log.warning(
            "not promoting: %d held-out rows, %d are needed for the comparison to mean anything",
            len(held_out),
            MIN_HELD_OUT,
        )
        promote = False
    else:
        promote = beats(metrics, incumbent["metrics"] if incumbent else None)

    # Content, not a counter: two models trained on the same *number* of labels
    # are different models, and a version string that collides makes the
    # registry's ON CONFLICT overwrite one with the other.
    fingerprint = hashlib.sha256(
        json.dumps(
            {k: artifact[k] for k in ("vocabulary", "centroids", "categories")},
            sort_keys=True,
        ).encode()
    ).hexdigest()[:16]
    version = f"{artifact['version']}+{fingerprint}"
    log.info(
        "candidate %s: accuracy=%.3f coverage=%.3f on %d held out; incumbent=%s; promote=%s",
        version,
        metrics["accuracy"],
        metrics["coverage"],
        metrics["n"],
        incumbent["model_version"] if incumbent else "none",
        promote,
    )

    with conn.transaction():
        # An existing row is never edited: re-running with identical data is a
        # no-op, and a losing candidate must not overwrite the active model's
        # artifact. Promotion is a flag change, done separately.
        conn.execute(
            """
            INSERT INTO model_registry (model_version, trained_at, metrics, active, artifact)
            VALUES (%s, NOW(), %s, FALSE, %s)
            ON CONFLICT (model_version) DO NOTHING
            """,
            (
                version,
                json.dumps({**metrics, "training_size": len(training)}),
                json.dumps(artifact),
            ),
        )
        if promote:
            # One active model at a time, enforced by a unique index; the old one
            # is demoted first so the index never sees two.
            conn.execute("UPDATE model_registry SET active = FALSE WHERE active")
            conn.execute(
                "UPDATE model_registry SET active = TRUE WHERE model_version = %s",
                (version,),
            )

    return {"model_version": version, "metrics": metrics, "active": promote}


def main() -> int:
    logging.basicConfig(stream=sys.stdout, level=logging.INFO)
    url = db.database_url()
    if not url:
        log.error("ANALYTICS_DB_URL is not set, so there is nothing to train on")
        return 1
    # Training reads every tenant's purchases on purpose, so it cannot run as
    # the RLS-scoped `analytics` role: with no tenant set, that role sees nothing
    # and the job would report "not enough data" forever. The owner URL is
    # required and has no fallback -- a silent empty training set is the worst
    # possible outcome here.
    owner_url = os.environ.get(TRAIN_DB_URL_ENV)
    if not owner_url:
        log.error(
            "%s is not set: training needs a role that can read across tenants",
            TRAIN_DB_URL_ENV,
        )
        return 1
    with psycopg.connect(owner_url) as conn:
        result = run(conn)
    if result is None:
        return 2
    log.info("registered %s (active=%s)", result["model_version"], result["active"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
