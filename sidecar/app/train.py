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

import json
import logging
import sys

import psycopg

from app import db, model

log = logging.getLogger(__name__)

# The oldest 80% trains, the newest 20% tests. The test set is the future, which
# is the only honest thing to test a forecast-like model on.
TRAIN_FRACTION = 0.8

METRICS = ("accuracy", "coverage")


def labelled_purchases(conn: psycopg.Connection) -> list[tuple[str, int, str]]:
    """Purchases the user categorized, oldest first (D63: the settled value).

    The label is `purchase.category_id`, because that is the value the user
    finally left in place. A model trained on the sidecar's own predictions
    would learn to reproduce itself.
    """
    rows = conn.execute(
        """
        SELECT p.id, p.category_id,
               COALESCE(p.store, '') || ' ' || COALESCE(p.description, '') AS text,
               p.purchase_date
          FROM purchase p
         WHERE p.category_id IS NOT NULL
           AND p.deleted_at IS NULL
         ORDER BY p.purchase_date, p.created_at
        """
    ).fetchall()
    return [(row[2], row[1], row[3]) for row in rows]


def active_model(conn: psycopg.Connection) -> dict | None:
    row = conn.execute(
        "SELECT model_version, metrics FROM model_registry WHERE active"
    ).fetchone()
    if row is None:
        return None
    metrics = row[1] if isinstance(row[1], dict) else json.loads(row[1])
    return {"model_version": row[0], "metrics": metrics}


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
    promote = beats(metrics, incumbent["metrics"] if incumbent else None)

    version = f"{artifact['version']}+{len(training)}"
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
        if promote and incumbent is not None:
            # One active model at a time, enforced by a unique index; the old one
            # is demoted first so the index never sees two.
            conn.execute("UPDATE model_registry SET active = FALSE WHERE active")
        conn.execute(
            """
            INSERT INTO model_registry (model_version, trained_at, metrics, active, artifact)
            VALUES (%s, NOW(), %s, %s, %s)
            ON CONFLICT (model_version) DO UPDATE
                SET trained_at = NOW(), metrics = EXCLUDED.metrics,
                    artifact = EXCLUDED.artifact
            """,
            (
                version,
                json.dumps({**metrics, "training_size": len(training)}),
                promote,
                json.dumps(artifact),
            ),
        )

    return {"model_version": version, "metrics": metrics, "active": promote}


def main() -> int:
    logging.basicConfig(stream=sys.stdout, level=logging.INFO)
    url = db.database_url()
    if not url:
        log.error("ANALYTICS_DB_URL is not set, so there is nothing to train on")
        return 1
    with psycopg.connect(url) as conn:
        result = run(conn)
    if result is None:
        return 2
    log.info("registered %s (active=%s)", result["model_version"], result["active"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
