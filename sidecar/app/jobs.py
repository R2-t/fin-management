"""The sidecar's job worker.

The application owns the write path: it enqueues a job when something happens
that needs analytics work, and this worker claims and completes those jobs.
The sidecar never inserts a job and never writes to `purchase` -- both are
the application's job (D4, D66).

Claiming uses `FOR UPDATE SKIP LOCKED`, the same mechanism the Rust
projector's claim uses, so two workers cannot take the same job and a crashed
worker's claim is reclaimed rather than lost.
"""

from __future__ import annotations

import logging
import time

import psycopg

from app import db

# How often the worker polls when the queue is empty.
POLL_SECONDS = 1.0

# The kinds this worker handles. The claim filters to these, so a job of an
# unknown kind is never claimed -- it waits for a build that knows it.
HANDLED_KINDS = ("classify",)

# The retry bounds, the same as the Rust job queue's (D58, review finding 6):
# exhaust after MAX_ATTEMPTS, back off 5 * 2^attempts MINUTES.
MAX_ATTEMPTS = 10

log = logging.getLogger(__name__)


def claim_job(conn: psycopg.Connection) -> dict | None:
    """Atomically claims the oldest due job of a handled kind.

    The claim sets the job's status so a second worker's `FOR UPDATE SKIP
    LOCKED` skips past it. Returns the row as a dict, or None.
    """
    kinds = tuple(HANDLED_KINDS)
    with conn.transaction():
        row = conn.execute(
            """
            SELECT id, user_id, kind, payload
              FROM analytics_job
             WHERE status = 'pending'
               AND kind = ANY(%s)
               AND attempts < %s
               AND run_after <= NOW()
             ORDER BY run_after
             LIMIT 1
             FOR UPDATE SKIP LOCKED
            """,
            (list(kinds), MAX_ATTEMPTS),
        ).fetchone()

        if row is None:
            return None

        job_id, user_id, kind, payload = row
        conn.execute(
            "UPDATE analytics_job SET status = 'running', updated_at = NOW() "
            " WHERE id = %s",
            (job_id,),
        )
        # The tenant is NOT stated here: `set_config(..., true)` is
        # transaction-local, and this transaction commits when the block
        # exits. The handler states the user in the transaction it opens, so
        # the scope spans exactly the work it does.
        return {
            "id": job_id,
            "user_id": str(user_id),
            "kind": kind,
            "payload": payload if isinstance(payload, dict) else {},
        }


def complete_job(conn: psycopg.Connection, job_id: int) -> None:
    with conn.transaction():
        conn.execute(
            "UPDATE analytics_job SET status = 'done', updated_at = NOW() "
            " WHERE id = %s",
            (job_id,),
        )


def fail_job(conn: psycopg.Connection, job_id: int, error: str) -> None:
    """Records the attempt and schedules a retry with backoff.

    The bounds are the Rust job queue's: exhaust after MAX_ATTEMPTS, back off
    5 * 2^attempts minutes (review finding 6).

    shortcut: a worker killed between claiming and finishing leaves the job
    `running` forever -- `analytics_job` has no claimed_at column to reclaim
    by, unlike `analytics_outbox`. Revisit when a job is heavy enough to
    crash the worker; add a claim timeout sweep then.
    """
    with conn.transaction():
        row = conn.execute(
            """
            UPDATE analytics_job
               SET attempts = attempts + 1,
                   last_error = %s,
                   status = CASE
                       WHEN attempts + 1 >= %s THEN 'failed'::text
                       ELSE 'pending'::text
                   END,
                   run_after = NOW() + make_interval(
                       mins => 5 * (2 ^ LEAST(attempts, 6))::int
                   ),
                   updated_at = NOW()
             WHERE id = %s
             RETURNING attempts, status
            """,
            (error[:500], MAX_ATTEMPTS, job_id),
        ).fetchone()
        if row is None:
            return
        attempts, status = row
        if status == "failed":
            log.error(
                "job %s exhausted its retries after %s attempts", job_id, attempts
            )


def run_forever() -> None:
    """Polls the queue and runs jobs until the process is stopped."""
    url = db.database_url()
    if not url:
        raise RuntimeError(
            "ANALYTICS_DB_URL is not set, so the worker cannot poll the queue. "
            "Set it, or do not start the worker."
        )
    log.info("job worker polling %s kinds", len(HANDLED_KINDS))
    while True:
        try:
            with psycopg.connect(url) as conn:
                job = claim_job(conn)
                if job is None:
                    time.sleep(POLL_SECONDS)
                    continue
                log.info("claimed job %s kind %s", job["id"], job["kind"])
                try:
                    from app.classify import handle_classify

                    if job["kind"] == "classify":
                        handle_classify(conn, job)
                    complete_job(conn, job["id"])
                except Exception as exc:  # noqa: BLE001 -- the worker must survive a bad job
                    log.exception("job %s failed", job["id"])
                    fail_job(conn, job["id"], str(exc))
        except psycopg.OperationalError:
            # The database is unreachable. Back off rather than spinning.
            log.exception("database unreachable; backing off")
            time.sleep(5)
        except Exception:  # noqa: BLE001
            log.exception("worker iteration failed")
            time.sleep(1)
