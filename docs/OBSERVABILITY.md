# Observability reference

Full audit: `docs/reports/observability-review.md`.

## Conventions
- Metrics are emitted as tracing events (`monotonic_counter.*`, `histogram.*`, `gauge.*`); the prefix is stripped
  and the collector adds `finmgmt_`. Metric events carry **bounded labels only** (never error text, ids, raw paths);
  details go in a separate log event.
- Console logs: `LOG_FORMAT=json` for JSON, `RUST_LOG` for filtering. OTLP export of traces/logs/metrics continues in parallel.
- Tracing: services/repos use `#[instrument(skip_all, err(level="warn"))]` so payloads are never recorded; 5xx marks the request span `ERROR`.
  W3C `traceparent` is extracted on inbound requests; **outbound injection is not implemented** (no outbound HTTP client until the Python sidecar). `TRACE_SAMPLE_RATIO` (default 1.0), `OTEL_METRIC_EXPORT_INTERVAL` (ms).

## Metrics
| Metric | Labels | Meaning |
|---|---|---|
| `http_requests_total` | http_route, http_request_method, http_response_status | RED rate/errors |
| `http_request_duration_ms` (hist) | same | RED duration |
| `http_5xx_total` | same | 5xx count |
| `internal_errors_total` | kind | unexpected 500s (cause in logs) |
| `repository_errors_total` | kind=db_error/pool_timeout/pool_connect/serde | DB-layer failures |
| `db_pool_connections` (gauge) | pool_state=idle/in_use | pool saturation |
| `auth_failures_total` | reason | missing_header/invalid_token/unknown_key/key_provider/internal |
| `fx_provider_failure_total` | kind | provider errors |
| `fx_provider_duration_ms` (hist) | outcome | provider latency |
| `fx_rate_cache_total` | result=hit/miss | cache hit ratio |
| `fx_stale_fallback_total` | – | purchases using stale rates |
| `panics_total` | – | handler panics |
| `purchase_created_total` | currency, rate_source | business throughput |

The `purchase_created_total` label `rate_source` is collapsed to `none|manual|api|other` so provider text cannot mint label values.

Collector is pinned to `otel/opentelemetry-collector-contrib:0.159.0` (the `resource_to_telemetry_conversion` option that adds `service_name` was verified on it). Validate any change offline with `scripts/validate-observability.sh` (collector `validate`, `promtool`, `amtool`).

Alerts: `prometheus/alerts.yml` (routing: `prometheus/alertmanager.yml`, receiver is a placeholder).
Validate with `promtool check rules` / `amtool check-config`.

## Known gaps
- Optimistic-lock conflict counter, DB query latency histograms, and outbox/Python sidecar metrics (see review §6–7) are not yet implemented.
- **Alertmanager receiver is still a placeholder** (`localhost:9999`): fired alerts are dropped until a real receiver (Slack/email/PagerDuty) is configured.
- `HighHttp5xxRatio` stays silent until the first 5xx exists, so "no errors" and "no traffic" look the same.
