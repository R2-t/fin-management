#!/bin/sh
set -e

# Apply DB migrations before serving traffic. The migrations binary reads
# DB_HOST plus DB_USERNAME/DB_PASSWORD (falling back to DEFAULT_USER/
# DEFAULT_PASSWORD). Skipped when RUN_MIGRATIONS is explicitly "0".
if [ "${RUN_MIGRATIONS:-1}" != "0" ]; then
    echo "running migrations..."
    /usr/local/bin/migrations
fi

exec /usr/local/bin/financial-management