#!/usr/bin/env bash
# Validates collector, Prometheus rules and Alertmanager config offline.
# Run after touching otel-collector-config.yaml or prometheus/*.
set -euo pipefail
cd "$(dirname "$0")/.."
RUNTIME=${CONTAINER_RUNTIME:-podman}
COLLECTOR_IMAGE=$(grep -o 'otel/opentelemetry-collector-contrib:[^ ]*' Local.DockerCompose.yaml | head -1)

$RUNTIME run --rm -v "$PWD/otel-collector-config.yaml:/c.yaml" "$COLLECTOR_IMAGE" validate --config=/c.yaml
$RUNTIME run --rm -v "$PWD/prometheus:/p" --entrypoint promtool prom/prometheus:latest check rules /p/alerts.yml
$RUNTIME run --rm -v "$PWD/prometheus:/p" --entrypoint amtool prom/alertmanager:latest check-config /p/alertmanager.yml
echo "observability config OK ($COLLECTOR_IMAGE)"
