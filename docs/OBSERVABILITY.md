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
| `analytics_outbox_lag_seconds` (gauge) | – | age of the oldest unprocessed outbox row |
| `fact_projected_total` | – | fact rows rebuilt |
| `outbox_listener_connected` (gauge) | – | 1 while the `LISTEN` connection is up, 0 after it drops |
| `outbox_listener_reconnects_total` | – | `LISTEN` connection reconnects |
| `purchase_created_total` | currency, rate_source | business throughput |

The `purchase_created_total` label `rate_source` is collapsed to `none|manual|api|other` so provider text cannot mint label values.

Collector is pinned to `otel/opentelemetry-collector-contrib:0.159.0` (the `resource_to_telemetry_conversion` option that adds `service_name` was verified on it). Validate any change offline with `scripts/validate-observability.sh` (collector `validate`, `promtool`, `amtool`).

Alerts: `prometheus/alerts.yml` (routing: `prometheus/alertmanager.yml`, receiver is a placeholder).
Validate with `promtool check rules` / `amtool check-config`.

## Analytics projection

`purchase`/`item` mutations insert an `analytics_outbox` row in the same
transaction and signal `pg_notify('analytics_outbox', '')`; the notification is
transactional, so it fires on COMMIT only. A dedicated `LISTEN` connection
(`repository::analytics::listen_forever`) wakes the projector, and the 1 s poll
remains as a **mandatory fallback** because notifications are not queued for
absent listeners and a half-open socket can swallow them.

The listener cannot use the bb8 pool: `bb8-postgres` drives the `Connection`
future internally and discards `Notification` messages, which tokio-postgres only
surfaces via `Connection::poll_message`. The listener therefore owns its own
connection and its own reconnect loop (exponential backoff, reset after a
connection that stayed up), with TCP keepalives set in
`config::postgresconfig::Config::get_config`.

Measured over repeated samples: **~22-27 ms** projection lag via the doorbell, vs **~145-1011 ms**
for rows inserted without `pg_notify` (i.e. an unremarkable roll of the 1 s poll).
The poll remains the safety net, not the fast path.

## Known gaps
- Optimistic-lock conflict counter and DB query latency histograms (review §6–7) are not yet implemented; Python-sidecar job metrics arrive with Phase 3.
- **Alertmanager receiver is still a placeholder** (`localhost:9999`): fired alerts are dropped until a real receiver (Slack/email/PagerDuty) is configured.
- `HighHttp5xxRatio` stays silent until the first 5xx exists, so "no errors" and "no traffic" look the same.
