"""The training job (3.7), against the real database.

It needs ANALYTICS_DB_URL like the rest of the database tests; without it the
module is skipped explicitly rather than pretending to pass.
"""

import json
import os
import uuid

import psycopg
import pytest

from app import train

pytestmark = pytest.mark.skipif(
    not os.environ.get("ANALYTICS_DB_URL"),
    reason="needs ANALYTICS_DB_URL",
)


@pytest.fixture
def owner():
    """The owner connection: the job writes the registry, the sidecar reads it."""
    with psycopg.connect(os.environ["TEST_DATABASE_URL"]) as conn:
        yield conn


def seed_labels(owner, user, count=40):
    """Seeded through the owner because the sidecar cannot write `purchase`."""
    rows = []
    categories = []
    for name in ("COFFEE", "GASOLINE", "STREAMING"):
        row = owner.execute(
            "INSERT INTO category (user_id, name) VALUES (%s, %s) RETURNING id",
            (user, f"{name.lower()}-{user}"),
        ).fetchone()
        categories.append(row[0])

    wallet = owner.execute(
        "INSERT INTO wallet (id, user_id, name, currency) VALUES (%s, %s, %s, 'USD') RETURNING id",
        (str(uuid.uuid4()), user, f"w-{user}"),
    ).fetchone()[0]

    per_category = count // len(categories)
    for index, category in enumerate(categories):
        for n in range(per_category):
            store = ["BLUE BOTTLE COFFEE", "SHELL GAS STATION", "NETFLIX.COM"][index]
            row = owner.execute(
                """
                INSERT INTO purchase (id, user_id, wallet_id, category_id, store, currency,
                                      purchase_date)
                VALUES (%s, %s, %s, %s, %s, 'USD', NOW() - (%s || ' days')::interval)
                RETURNING id
                """,
                (str(uuid.uuid4()), user, wallet, category, f"{store} {n}", n),
            ).fetchone()
            rows.append(row[0])
    owner.commit()
    return rows, categories


def test_running_twice_never_leaves_two_active_models(owner):
    """The invariant the unique index protects, through the job that could break
    it: two runs, one active model, and the second run is a no-op for the
    candidate it already registered."""
    first = train.run(owner)
    second = train.run(owner)

    active = owner.execute(
        "SELECT COUNT(*) FROM model_registry WHERE active"
    ).fetchone()[0]
    assert active <= 1, "at most one model can be active"

    if first is not None and second is not None:
        assert first["model_version"] == second["model_version"], (
            "identical data produces the same content hash, so the second run "
            "registers nothing new"
        )


def test_a_registered_model_is_loadable_and_the_gate_decides_active(owner):
    user = str(uuid.uuid4())
    seed_labels(owner, user)

    result = train.run(owner)
    if result is None:
        pytest.skip("not enough labelled history in this database to train")

    row = owner.execute(
        "SELECT metrics, artifact, active FROM model_registry WHERE model_version = %s",
        (result["model_version"],),
    ).fetchone()
    assert row is not None, "the candidate is registered even when not promoted"

    metrics, artifact, active = row
    assert metrics["accuracy"] >= 0.0
    assert "vocabulary" in artifact and "centroids" in artifact
    # Cross-user: the classes are category NAMES, not ids, because ids belong to
    # the user who created the category.
    assert all(isinstance(c, str) for c in artifact["categories"]), artifact["categories"]
    # A row promoted by an earlier run keeps its flag (inserts never edit an
    # existing row), so the assertion is one-way.
    if result["active"]:
        assert active is True, "a promoted candidate is active"


