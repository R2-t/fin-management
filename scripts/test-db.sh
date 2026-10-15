#!/usr/bin/env bash
# Starts a throwaway TimescaleDB for the integration tests and prints the DSN.
#
# The integration tests fail loudly when TEST_DATABASE_URL is unset, so this
# script is the supported way to satisfy them.
#
#   ./scripts/test-db.sh          # start (or reuse) and print the DSN
#   ./scripts/test-db.sh --stop   # stop and remove the container
set -euo pipefail
cd "$(dirname "$0")/.."

RUNTIME=${CONTAINER_RUNTIME:-podman}
NAME=${TEST_DB_CONTAINER:-finmgmt-test-db}
PORT=${TEST_DB_PORT:-55432}
IMAGE=timescale/timescaledb:2.23.0-pg18
DSN="postgres://postgres:postgres@127.0.0.1:${PORT}/postgres"

if [[ "${1:-}" == "--stop" ]]; then
  "$RUNTIME" rm -f "$NAME" >/dev/null 2>&1 || true
  echo "stopped $NAME"
  exit 0
fi

if ! $RUNTIME ps --format '{{.Names}}' | grep -qx "$NAME"; then
  $RUNTIME run -d --rm --name "$NAME" -e POSTGRES_PASSWORD=postgres -p "${PORT}:5432" "$IMAGE" >/dev/null
  echo "starting $NAME..." >&2
fi

# Wait for readiness by actually connecting: initdb plus the extension can take
# well over half a minute on a first run.
ready=0
for _ in $(seq 1 60); do
  if $RUNTIME exec "$NAME" psql -U postgres -tAc 'select 1' >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 2
done
if [[ "$ready" != 1 ]]; then
  echo "database did not become ready; recent container logs:" >&2
  $RUNTIME logs --tail 20 "$NAME" >&2 || true
  exit 1
fi

# Apply migrations in version order (a plain `U*.sql` glob sorts U10 before U2).
for f in $(ls migrations/migrations/U*.sql | sort -V); do
  $RUNTIME exec -i "$NAME" psql -U postgres -q -v ON_ERROR_STOP=1 <"$f" >/dev/null
done

echo "export TEST_DATABASE_URL='$DSN'"
