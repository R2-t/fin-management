"""Database access for the sidecar.

Connects as the read-only `analytics` role (D4): it can read what it needs to
classify and write only the tables it owns, so a compromised sidecar cannot
corrupt a purchase, a ledger entry or a balance.

Matching is skipped entirely when ANALYTICS_DB_URL is unset, so the service
runs -- and its tests run -- without a database. A configured database that
fails is a 500, not a silent "no match": the caller must be able to tell
"there is no merchant" from "I could not check".
"""

import os

import psycopg

DATABASE_URL_ENV = "ANALYTICS_DB_URL"


def database_url() -> str | None:
    return os.environ.get(DATABASE_URL_ENV)


def aliases_for_user(user_id: str) -> list[tuple[str, str]]:
    """The user's recorded merchant aliases as `(alias, merchant_id)` pairs.

    Owned by the caller so the matching function stays free of a database.
    """
    url = database_url()
    if not url:
        return []

    with psycopg.connect(url) as conn:
        rows = conn.execute(
            """
            SELECT ma.alias, ma.merchant_id::text
              FROM merchant_alias ma
              JOIN merchant m ON m.id = ma.merchant_id
             WHERE m.user_id = %s::uuid
               AND m.deleted_at IS NULL
             ORDER BY ma.alias
            """,
            (user_id,),
        ).fetchall()

    return [(row[0], row[1]) for row in rows]
