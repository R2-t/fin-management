# Observability Review — financial-management

**Date:** 2025-10-01
**Scope:** `src/main.rs`, `src/config/otlpconfig.rs`, `src/api/**`, `src/auth/**`,
`src/application/**`, `src/repository/**`, `otel-collector-config.yaml`,
`prometheus/prometheus.yml`, `prometheus/alerts.yml`, `Local.DockerCompose.yaml`,
`scripts/`.
**Constraint:** read-only audit — no code was modified; this report is the only
artifact.

---

## 1. Executive summary

The repo has the *plumbing* for all three signals (OTLP traces, logs, metrics)
but the observability story is **not production-usable yet**:

1. **Alerts can never fire.** Every PromQL rule in `prometheus/alerts.yml`
   references a metric name that does not exist, because
   `tracing-opentelemetry`'s metrics layer strips the `counter.`/`histogram.`/
   `monotonic_counter.` prefix from the field name. This is a hard, silent
   failure (P0).
2. **Internal errors are swallowed.** `ApplicationError::Internal`/
   `Serialization` are collapsed to an opaque `ApiError::Internal`, and nothing
   logs or counts them. DB failures, pool exhaustion, FX errors and serde
   failures all surface as a bare `500` with zero diagnostics (P0).
3. **No Alertmanager and no `alerting:` block** in Prometheus, so even correct
   rules would not be routed anywhere (P0).
4. **PII/financial data leaks into spans.** Services use
   `#[instrument(skip(self))]` only, so the whole command structs (user ids,
   wallet ids, store/notes, item names, unit prices) are recorded as span
   attributes and shipped to Jaeger (P1).
5. **Repositories and the FX call have no spans, no DB pool metrics, and no
   query latency/failure metrics** — the most useful signal for a DB-bound
   service is missing (P1).
6. **Distributed trace context is never propagated** (no W3C propagator is
   registered, no header extract/inject), so traces cannot cross process
   boundaries — directly blocking the planned Rust↔Python sidecar (P1).

The good news: the layering in `config::Config::init` is a good foundation, the
request span already carries `http.response.status`, and metrics are emitted as
structured events, so fixes are mostly additive.

---

## 2. Signal inventory (what exists today)

### 2.1 Logging

| Location | What happens |
|---|---|
| `src/config/mod.rs:53-75` | registry = `filter` + `tracing_opentelemetry` (traces) + `OpenTelemetryTracingBridge` (logs) + `MetricsLayer` (metrics). **No `fmt` layer.** |
| `src/config/mod.rs:60-68` | Hard-coded dynamic filter: INFO spans + INFO events only. No `RUST_LOG`, no DEBUG/TRACE, no per-target control. |
| `src/main.rs:105-108` | `tracing::info!(histogram.http_request_duration_ms=…, http.response.status=…)` on every response. Runs inside the request span scope (`tower-http` enters the span before `on_response`), so trace/span ids are correlated. |
| `src/main.rs:112-115` | `tracing::warn!(counter.http_5xx_total=1, …)` for 5xx. |
| `src/application/purchase/service.rs:87-91` | `monotonic_counter.purchase_created_total` + `currency`/`rate_source` attrs. |
| `src/application/purchase/service.rs:110-115` | `warn!` manual FX override (logs the rate). |
| `src/application/purchase/service.rs:171-176` | `monotonic_counter.fx_provider_failure_total` + currency pair attrs. |
| `src/application/purchase/service.rs:182-188` | `warn!` stale-rate fallback. |
| `src/auth/**` | **No logs at all** on auth failure. |
| `src/api/mod.rs:52-78` | **No log** when mapping `ApplicationError` → `ApiError`. |

Net effect: the only durable log sink is Loki (via OTLP). `make logs`
(`docker logs app`) is empty, and if the collector/Loki are down the service is
effectively mute. Every metric event is *also* exported as a log record by the
bridge, so Loki gets one noisy line per HTTP request.

### 2.2 Traces

- Request span: `src/main.rs:90-97` (`http.request.method/host/url`,
  `http.response.status`, `otel.name`).
- Service spans: `#[instrument(skip(self), fields(…))]` on all methods of
  `PurchaseService` (`service.rs:57,199,209,229,258,272,330,350`),
  `WalletService` (`wallet/service.rs:21,35,40,49,64`),
  `CategoryService` (`category/service.rs:21,39,47`),
  `PurchaseHistoryService` (`purchase_history/service.rs:24`).
- **No spans** on repositories/DB calls, on `ExchangeRateProvider::fetch_rate`,
  or on the auth middleware/verifier.
- **No error recording**: none of the `#[instrument]` attributes include
  `err`/`err(Debug)`, so failed calls are exported as successful spans.
- Sampler: `Sampler::AlwaysOn` (`otlpconfig.rs:74`).
- `otel.name` uses the **concrete** request path (`main.rs:96`), e.g.
  `Request GET /fin-management/purchase/<uuid>` → high-cardinality span names.

### 2.3 Metrics (actual exported names)

`tracing-opentelemetry` strips the prefix before creating the OTel instrument
(confirmed in `tracing-opentelemetry-0.30.0/src/metrics.rs:189-252`).
Non-prefixed fields on the same event become metric attributes/labels.

| Emitted field | OTel metric | Instrument | Expected Prometheus series (`namespace: finmgmt`) |
|---|---|---|---|
| `histogram.http_request_duration_ms` (`main.rs:106`) | `http_request_duration_ms` | f64 Histogram | `finmgmt_http_request_duration_ms_bucket/_sum/_count` |
| `counter.http_5xx_total` (`main.rs:113`) | `http_5xx_total` | **i64 UpDownCounter** | `finmgmt_http_5xx_total` (gauge) |
| `monotonic_counter.purchase_created_total` (`service.rs:88`) | `purchase_created_total` | u64 Counter | `finmgmt_purchase_created_total` |
| `monotonic_counter.fx_provider_failure_total` (`service.rs:172`) | `fx_provider_failure_total` | u64 Counter | `finmgmt_fx_provider_failure_total` |

Attributes currently attached: `http_response_status` (histogram + 5xx),
`currency`, `rate_source` (purchase counter), `source_currency`/`wallet_currency`
(FX failure counter). **No `route`/`method` label** on the HTTP metrics.

*(Confirm exact `_total`/unit handling by curling the collector:
`curl -s http://localhost:8889/metrics | grep finmgmt`. The prefix-stripping
itself is certain from the crate source.)*

### 2.4 Alerting

`prometheus/alerts.yml` has 4 rules, all with **incorrect metric names**:

| Rule | Line | Expression references | Should reference |
|---|---|---|---|
| `HighHttp5xxRate` | 7 | `finmgmt_counter_http_5xx_total` | `finmgmt_http_5xx_total` (and prefer monotonic) |
| `HighRequestLatency` | 20 | `finmgmt_histogram_http_request_duration_ms_bucket` | `finmgmt_http_request_duration_ms_bucket` |
| `FxProviderFailures` | 33 | `finmgmt_monotonic_counter_fx_provider_failure_total` | `finmgmt_fx_provider_failure_total` |
| `NoPurchasesCreated` | 45 | `finmgmt_monotonic_counter_purchase_created_total` | `finmgmt_purchase_created_total` |

`prometheus/prometheus.yml` has **no `alerting:` section** and there is **no
Alertmanager** service in `Local.DockerCompose.yaml`. Alerts are therefore
evaluated into the Prometheus UI only and never delivered.

---

## 3. Prioritized findings

### P0 — observability is non-functional

#### P0-1 — Alert rules reference metrics that do not exist (alerts never fire)
- **Where:** `prometheus/alerts.yml:7,20,33,45`
- **Why:** The metric names in the events are
  `histogram.http_request_duration_ms`, `counter.http_5xx_total`,
  `monotonic_counter.fx_provider_failure_total`,
  `monotonic_counter.purchase_created_total`. The metrics layer strips the
  prefix (`tracing-opentelemetry` `metrics.rs`), so the exported names do **not**
  contain `histogram_`, `counter_` or `monotonic_counter_`. The collector's
  `namespace: finmgmt` (`otel-collector-config.yaml:26`) prepends `finmgmt_`.
- **Fix:** Rewrite the expressions against the real series (verify with
  `curl -s http://localhost:8889/metrics | grep finmgmt`):
  ```promql
  # HighHttp5xxRate
  sum(rate(finmgmt_http_5xx_total[5m])) > 0.05
  # HighRequestLatency
  histogram_quantile(0.95,
    sum(rate(finmgmt_http_request_duration_ms_bucket[5m])) by (le)) > 1000
  # FxProviderFailures
  sum(increase(finmgmt_fx_provider_failure_total[5m])) > 0
  # NoPurchasesCreated
  sum(increase(finmgmt_purchase_created_total[15m])) == 0
  ```
  Longer term, stop hand-writing names: enable `resource_to_telemetry_conversion`
  (P1-8) and add a CI check that greps the exposed `/metrics` for every metric
  named in `alerts.yml`.

#### P0-2 — `ApplicationError::Internal` / `Serialization` are swallowed with no log and no metric
- **Where:** `src/api/mod.rs:52-63` (mapping) and `src/api/mod.rs:65-78`
  (`IntoResponse` returns a generic body). The originating cause is also
  flattened at `src/application/error.rs:26`
  (`other => ApplicationError::Internal(other.to_string())`).
- **Why:** Every `?` in a handler funnels through `From<ApplicationError>`. The
  `Internal` arm discards `msg`; `IntoResponse` emits
  `{"error":"Internal server error"}` and returns. Nothing calls
  `tracing::error!`, so DB outages, pool exhaustion, serde failures and FX
  exhaustion are invisible in logs, traces and metrics. This is the single
  biggest diagnostic gap.
- **Fix:** Log with the original error *before* collapsing it, and count it:
  ```rust
  impl From<ApplicationError> for ApiError {
      fn from(e: ApplicationError) -> Self {
          match e {
              // ... 4xx arms unchanged ...
              ApplicationError::Serialization(ref err) => {
                  tracing::error!(error = %err, "serialization failure");
                  tracing::warn!(counter.internal_errors_total = 1, kind = "serialization");
                  ApiError::Internal
              }
              ApplicationError::Internal(ref msg) => {
                  tracing::error!(error = %msg, "internal error");
                  tracing::warn!(counter.internal_errors_total = 1, kind = "internal");
                  ApiError::Internal
              }
          }
      }
  }
  ```
  Preserve classification: give `ApplicationError` distinct variants (or keep
  the `RepositoryError` source) so the log/metric can distinguish
  `pool_timeout` / `db_error` / `serde` / `fx`. Today `RepositoryError::DbError`
  and `RepositoryError::PoolError` are both stringified
  (`src/application/error.rs:19-28`), which is exactly what pool-exhaustion
  alerting needs.

#### P0-3 — No Alertmanager and no routing
- **Where:** `prometheus/prometheus.yml` (whole file — no `alerting:`),
  `Local.DockerCompose.yaml` (no `alertmanager` service).
- **Why:** Rules load via `rule_files` but have nowhere to go. `for: 5m` alerts
  simply flip `ALERTS` state.
- **Fix:** Add an `alertmanager` service and wire it:
  ```yaml
  # prometheus/prometheus.yml
  alerting:
    alertmanagers:
      - static_configs:
          - targets: ["alertmanager:9093"]
  ```
  Add a receiver (Slack/webhook/PagerDuty) in `prometheus/alertmanager.yml` and
  mount it in compose. Also add an `absent(up{job="otel-collector"})`/`up == 0`
  alert so a dead collector pages too.

---

### P1 — major gaps / production blockers

#### P1-1 — `#[instrument]` records entire command structs (PII + financial data)
- **Where:** `src/application/purchase/service.rs:57,229,258,272,330` (args
  `cmd`), `:209` (`filters`), `src/application/wallet/service.rs:21,49`
  (`name`), `src/application/category/service.rs:21` (`name`, `description`).
- **Why:** `skip(self)` skips only `self`. All other arguments are recorded with
  `Debug`, so spans exported to Jaeger/Loki contain `user_id`, `wallet_id`,
  `store`, `description`, `notes`, `item.name`, `unit_price`, quantities —
  personal and financial data. `docs/plan/05-hardening.md:2` explicitly targets
  "No PII in logs/traces".
- **Fix:** Use `skip_all` and re-add only safe identifiers/counts:
  ```rust
  #[instrument(skip_all, fields(user_id = %cmd.user_id, wallet_id = %cmd.wallet_id,
                                 item_count = cmd.items.len()))]
  pub async fn create(&self, cmd: CreatePurchaseCommand) -> Result<Purchase, ApplicationError> {
  ```
  For `list`, record `limit`/`offset` but not `store`. For wallet/category,
  `skip(name, description)` or `fields(name_len = name.len())`.

#### P1-2 — Errors are not recorded on spans / span status stays OK
- **Where:** all `#[instrument(...)]` sites listed in §2.2.
- **Why:** `tracing::instrument` does not set span status or record an error
  unless `err`/`err(Debug)` is present. A failed `create` looks like a healthy
  span in Jaeger, so error-rate-by-operation and tail-sampling of failures are
  impossible.
- **Fix:** `#[instrument(skip_all, err, ...)]` on every fallible service method
  (and on new repository spans, P1-4). This records the error event and sets
  `otel.status_code = ERROR` automatically.

#### P1-3 — No W3C trace-context propagation
- **Where:** no `set_text_map_propagator`, no header extract/inject anywhere
  (`src/main.rs`, `src/config/**`). Grep returns only `Sampler::AlwaysOn`.
- **Why:** OTel's global propagator defaults to a no-op, so an incoming
  `traceparent` is ignored and outgoing HTTP calls (the planned
  `HttpAnalyticsClient`) will not carry context. Traces stop at the process
  boundary.
- **Fix:** In `main`:
  ```rust
  opentelemetry::global::set_text_map_propagator(
      opentelemetry_sdk::propagation::TraceContextPropagator::new());
  ```
  Add an inbound middleware that extracts `traceparent`/`tracestate` into the
  request span's parent context, and set the same propagator on the outbound
  reqwest client (injector) for the sidecar. Keep the `TraceLayer` span but make
  it a child of the extracted remote span.

#### P1-4 — No repository/DB spans and no DB pool/query metrics
- **Where:** `src/repository/*.rs` — no `#[instrument]`, every method starts
  with `self.pool.get().await`. Pool is built with defaults at
  `src/config/postgresconfig.rs:37-41`.
- **Why:** For a DB-heavy service, DB latency, error rate and pool saturation
  are the top signals, and none exist. `bb8` exposes `Pool::state()`
  (`connections`/`idle_connections`) but it is never read.
- **Fix:**
  - Add `#[instrument(skip_all, err, fields(db.system="postgresql",
    db.operation="purchase.get", user_id=%user_id))]` to repo methods
    (exclude SQL parameters).
  - Instrument `pool.get()` and record acquire wait time.
  - Periodically sample `pool.state()` into gauges:
    `db_pool_connections{state="idle|used"}`, `db_pool_size` (configure
    `Pool::builder().max_size(...)` explicitly), plus
    `db_pool_acquire_timeouts_total` on `RepositoryError::PoolError`.
  - Emit `db_query_duration_ms` histogram per operation and
    `db_query_errors_total{operation,kind}`.

#### P1-5 — FX provider has no span/latency and the error is discarded
- **Where:** `src/application/purchase/service.rs:166-186` (`match self.fx.fetch_rate(req).await`),
  `src/application/outbound/exchange_rate.rs:51-84` (no `#[instrument]`).
- **Why:** Only a failure counter exists. There is no FX latency, no
  distinction between `ProviderUnavailable` / `PairNotFound` /
  `InvalidRate`, and `Err(_)` at `service.rs:170` throws away the cause. A real
  provider will be the main source of tail latency and external failure.
- **Fix:** Wrap the call in a span (`#[instrument(skip_all, fields(from, to))]`
  on `fetch_rate`) and emit `histogram.fx_provider_duration_ms` +
  `counter.fx_provider_errors_total{kind=...}`; change `Err(_)` to
  `Err(e) => { tracing::error!(error=%e, ...); … }` and label the failure
  counter with `kind`. Also count the stale-cache fallback
  (`fx_stale_fallback_total`).

#### P1-6 — Auth failures are neither logged nor metricized
- **Where:** `src/auth/mod.rs:44-71`; `src/auth/paseto.rs:38-82`;
  `src/auth/key_provider.rs`.
- **Why:** Missing header, invalid token, unknown kid, key-service 503 and
  internal verifier error all return a bare status/body. There is no
  `auth_failures_total{reason=…}`, no auth latency, and a key-service outage
  (503) is indistinguishable from a bad token in aggregate metrics.
- **Fix:** Emit `tracing::warn!(counter.auth_failures_total = 1, reason = "invalid_token")`
  in each `Err` arm (never log the token), and a span around `verifier.verify`
  with `histogram.auth_duration_ms`. Alert on `reason="key_provider"` separately
  (P0/P1 availability signal).

#### P1-7 — No stdout/console log layer; hard-coded INFO filter
- **Where:** `src/config/mod.rs:60-75` — registry has no
  `tracing_subscriber::fmt::layer()`; the filter ignores `RUST_LOG`.
- **Why:** `docker logs`/`make logs` show nothing from the app. When the
  collector is unavailable (exactly when you need logs most) there is no local
  copy. DEBUG (including `on_request`, `main.rs:99`) is always dropped.
- **Fix:** Add a console layer, e.g.
  `tracing_subscriber::fmt::layer().json()` in prod and pretty in dev, gated on
  an env var; base the filter on `EnvFilter::from_default_env()` (e.g.
  `RUST_LOG=info,financial_management=debug,tower_http=debug`). Keep the OTLP
  layers.

#### P1-8 — Collector does not promote `service.name` to a metric label
- **Where:** `otel-collector-config.yaml:22-25` (`prometheus` exporter).
- **Why:** Resource attributes are not converted to Prometheus labels by
  default. The alert rules group/annotate `by (service_name)`, but that label
  will be absent (only the `target_info` metric carries it). Alerts still fire
  but cannot break down per service, and `{{ $labels.service_name }}` renders
  empty.
- **Fix:**
  ```yaml
  exporters:
    prometheus:
      endpoint: 0.0.0.0:8889
      namespace: finmgmt
      resource_to_telemetry_conversion:
        enabled: true
  ```
  Then `service_name`, `service_namespace`, `service_version` become labels.

#### P1-9 — No per-route RED metrics
- **Where:** `src/main.rs:89-117`. The span/event only carries the concrete path
  and status; no `route`/`method` label on the histogram or 5xx counter.
- **Why:** Cannot answer "which endpoint got slow / is failing", the core of
  RED. Because `TraceLayer` sits *outside* routing, `MatchedPath` is not
  available at span creation.
- **Fix:** Add an inner middleware (after routing) that resolves
  `axum::extract::MatchedPath` and records `route`/`method` on the span, then
  include `route`/`method` as metric attributes in `on_response`. Alternatively
  instrument each handler and emit `counter.http_requests_total{route,method,status}`
  + `histogram.http_request_duration_ms{route,method}`. Keep `otel.name` as
  `"{method} {route_template}"` to fix P2-1 at the same time.

#### P1-10 — No panic handling or panic metric
- **Where:** `src/main.rs:120-123` (`bind(...).unwrap()`,
  `axum::serve(...).unwrap()`); no `CatchPanicLayer`, no panic hook.
- **Why:** A panic in a handler drops the connection with no log/metric and no
  consistent 500; a panic in startup aborts with only the default hook.
- **Fix:** `std::panic::set_hook` that logs `tracing::error!` and increments
  `counter.panics_total`, and add
  `tower_http::catch_panic::CatchPanicLayer::new()` to the router. Replace the
  startup `unwrap()`s with logged/`?` error handling.

---

### P2 — hygiene, cost, cardinality, polish

#### P2-1 — High-cardinality span name and host attribute
`src/main.rs:93,96`: `http.request.host = %host` and
`otel.name = "Request {method} {raw_path}"`. UUIDs in the path explode the
number of distinct operation names in Jaeger. Use the matched route template
(P1-9) and treat `http.request.host` as a resource/attribute, not a per-span
label.

#### P2-2 — `Sampler::AlwaysOn`
`src/config/otlpconfig.rs:74`: sample-everything will overwhelm the collector
and storage at any real volume. Use
`Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(ratio)))` with the
ratio from env, and keep error traces via tail sampling (collector
`tail_sampling` processor) once propagation exists.

#### P2-3 — `counter.http_5xx_total` is an UpDownCounter
`src/main.rs:113`: the `counter.` prefix produces an OTel UpDownCounter, which
Prometheus exports as a gauge. `rate()`/`increase()` semantics are wrong and it
cannot be treated as a monotonic total. Use
`monotonic_counter.http_5xx_total = 1`.

#### P2-4 — Metric events are duplicated into logs
The `OpenTelemetryTracingBridge` (`config/mod.rs:56`) exports *every* INFO event,
including the per-request metric pseudo-events (`main.rs:105`, `service.rs:87`,
`:171`). Every request writes a Loki line. Give metric events a distinct
`target` (e.g. `tracing::info!(target: "metrics", …)`) and filter the log bridge
to exclude `target = "metrics"`.

#### P2-5 — FX failure counter cardinality
`src/application/purchase/service.rs:171-176` labels the counter with both
`source_currency` and `wallet_currency` — O(currencies²) (≈180²). Prefer a
single `pair="USD/EUR"` label only if the set is small, or drop the pair and
keep per-provider labels.

#### P2-6 — Collector `debug` exporter in every pipeline
`otel-collector-config.yaml:19-21,45-57`: `debug` with `verbosity: detailed` in
traces/metrics/logs duplicates all data to collector stdout (cost + it will
print whatever PII leaked into spans). Remove it, or gate it behind a dev
config. Add a `filter`/`attributes` processor to drop/redact known-sensitive
attributes before Jaeger/Loki.

#### P2-7 — Weak resource attributes
`otlpconfig.rs:57-58,97-98,124-125` set `x-host`/`x-service-name` as gRPC
metadata (redundant, not queryable) while the resource only has `service.name`
(`otlpconfig.rs:74,104,132`). Add `service.version`,
`deployment.environment`, and stop stuffing the service name into custom `x-`
headers. `otel-collector-config.yaml:14-18` hard-codes `service.namespace`
which is better set as a resource.

#### P2-8 — Local stack reliability gaps
`Local.DockerCompose.yaml`: images `:latest` unpinned; no volumes for
Prometheus/Loki/Jaeger (telemetry lost on restart); `depends_on:
service_started` instead of `service_healthy` (early telemetry/migration races);
no `alertmanager`; no `RUST_LOG`/sampler env on `app`. The `health_check`
extension (`otel-collector-config.yaml:38-39`) is not used by any healthcheck.

#### P2-9 — No optimistic-lock conflict metric
`src/repository/purchase.rs:73-89` (`miss_reason`) and `:94-114`
(`bump_purchase_version`) distinguish `VersionConflict` from `NotFound`, but
nothing counts conflicts. Emit
`counter.db_optimistic_lock_conflicts_total{table}`. This is a key product
signal for a concurrent-edit UX.

#### P2-10 — Startup `unwrap()`s without context
`src/main.rs:120,123`, `src/config/postgresconfig.rs:34,40`,
`src/config/otlpconfig.rs:66,107,134`. Replace with logged errors so a startup
failure is diagnosable (and not a bare panic). Note the OTLP exporter build
`unwrap()`s happen *before* the subscriber is installed, so any failure there is
invisible.

#### P2-11 — Secrets/PII status (currently acceptable)
`.env` contains `DEFAULT_PASSWORD`, but `.gitignore` excludes `.env`; the DB
password is only used to build the connection string
(`postgresconfig.rs:23-35`) and is never logged. No `Authorization` header or
token is logged (`auth/mod.rs`), and `VerifyError` does not carry token bytes.
The main leakage is P1-1 (span args). Keep it that way: when adding DB spans,
never record SQL bind parameters (they include `store`, `notes`, amounts).

---

## 4. Proposed PromQL alert rules

Add to `prometheus/alerts.yml` (names assume P1-9's `route`/`method` labels
exist; drop those groupings until then). Confirm series names against
`/metrics` first.

```yaml
groups:
  - name: financial-management-slo
    rules:
      # ---- Availability / errors -------------------------------------------
      - alert: HighHttp5xxRate
        expr: |
          (
            sum(rate(finmgmt_http_5xx_total[5m]))
            /
            clamp_min(sum(rate(finmgmt_http_request_duration_ms_count[5m])), 0.001)
          ) > 0.05
        for: 5m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "5xx ratio > 5%"
          description: "Error ratio {{ $value | humanizePercentage }} over 5m."

      - alert: Endpoint5xx
        expr: |
          sum by (route, method) (rate(finmgmt_http_5xx_total[5m])) > 0.1
        for: 5m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "{{ $labels.method }} {{ $labels.route }} returning 5xx"

      # ---- Latency ----------------------------------------------------------
      - alert: HighRequestLatencyP95
        expr: |
          histogram_quantile(0.95,
            sum by (le, route, method)
              (rate(finmgmt_http_request_duration_ms_bucket[5m]))
          ) > 1000
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "P95 > 1s on {{ $labels.method }} {{ $labels.route }}"

      - alert: HighRequestLatencyP99
        expr: |
          histogram_quantile(0.99, sum(rate(finmgmt_http_request_duration_ms_bucket[5m])) by (le)) > 3000
        for: 5m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "P99 > 3s"

      # ---- DB pool exhaustion (requires P1-4 metrics) -----------------------
      - alert: DbPoolSaturation
        expr: |
          finmgmt_db_pool_used
            / clamp_min(finmgmt_db_pool_size, 1) > 0.9
        for: 5m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "DB pool > 90% used ({{ $value | humanizePercentage }})"

      - alert: DbPoolAcquireTimeouts
        expr: increase(finmgmt_db_pool_acquire_timeouts_total[5m]) > 0
        for: 1m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "DB pool acquire timeouts detected"

      - alert: DbPoolAcquireLatencyHigh
        expr: |
          histogram_quantile(0.95,
            sum(rate(finmgmt_db_pool_acquire_duration_ms_bucket[5m])) by (le)
          ) > 250
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "P95 DB acquire wait > 250ms"

      - alert: DbQueryErrors
        expr: sum by (operation) (rate(finmgmt_db_query_errors_total[5m])) > 0.05
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "DB errors on {{ $labels.operation }}"

      # ---- FX --------------------------------------------------------------
      - alert: FxProviderFailures
        expr: sum(increase(finmgmt_fx_provider_failure_total[5m])) > 3
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "FX provider failures: {{ $value }} in 5m"

      - alert: FxProviderLatencyHigh
        expr: |
          histogram_quantile(0.95,
            sum(rate(finmgmt_fx_provider_duration_ms_bucket[5m])) by (le)
          ) > 2000
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "P95 FX provider latency > 2s"

      - alert: FxStaleRateFallback
        expr: increase(finmgmt_fx_stale_fallback_total[5m]) > 0
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "Serving stale FX rates"

      # ---- Auth ------------------------------------------------------------
      - alert: AuthFailuresHigh
        expr: |
          sum(increase(finmgmt_auth_failures_total{reason=~"invalid_token|key_not_found"}[5m])) > 50
        for: 5m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "High auth failure volume"

      - alert: AuthKeyProviderUnavailable
        expr: increase(finmgmt_auth_failures_total{reason="key_provider"}[5m]) > 0
        for: 2m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "Token key service unreachable — all auth failing"

      # ---- Correctness / runtime -------------------------------------------
      - alert: PanicsDetected
        expr: increase(finmgmt_panics_total[5m]) > 0
        for: 1m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "Handler panic detected"

      - alert: OptimisticLockContention
        expr: sum(rate(finmgmt_db_optimistic_lock_conflicts_total[5m])) > 0.5
        for: 10m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "Elevated version conflicts ({{ $value }}/s)"

      - alert: InternalErrors
        expr: sum(rate(finmgmt_internal_errors_total[5m])) > 0.1
        for: 5m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "Server-side internal errors"

      # ---- Cache efficiency (requires P1-4/§6 counter) ---------------------
      - alert: FxCacheHitRatioLow
        expr: |
          (
            sum(rate(finmgmt_rate_cache_hits_total[15m]))
            /
            clamp_min(sum(rate(finmgmt_rate_cache_hits_total[15m]))
                    + sum(rate(finmgmt_rate_cache_misses_total[15m])), 0.001)
          ) < 0.5
        for: 30m
        labels: { severity: warning, team: payments }
        annotations:
          summary: "FX cache hit ratio < 50%"

      # ---- Telemetry pipeline health ---------------------------------------
      - alert: TelemetryCollectorDown
        expr: up{job="otel-collector"} == 0
        for: 5m
        labels: { severity: critical, team: payments }
        annotations:
          summary: "OTel collector scrape target down"
```

---

## 5. Collector / pipeline review (`otel-collector-config.yaml`)

| Issue | Line(s) | Fix |
|---|---|---|
| No `resource_to_telemetry_conversion` → `service_name` missing on metrics | 22-25 | Add `resource_to_telemetry_conversion: {enabled: true}` (P1-8) |
| `debug` exporter with `verbosity: detailed` in all 3 pipelines | 19-21, 45-57 | Remove or gate to a dev-only config; it is the only place PII-bearing spans get printed |
| No PII/high-cardinality scrubbing before Jaeger/Loki | processors 7-18 | Add `attributes`/`filter`/`transform` to drop/redact sensitive attrs; add a `tail_sampling` processor for error traces |
| `service.namespace` hard-coded via attributes processor | 14-18 | Set it as a resource attribute in the app (with `service.version`, `deployment.environment`) |
| `debug` metrics detail on collector itself | 60-64 | `detailed` is fine for the collector's own metrics but noisy; keep normal unless debugging |
| `health_check` defined but not used by compose | 38-39 | Add container healthcheck using `:13133` |
| Receiver bound to `0.0.0.0` | 5,8 | Expected in container; restrict via network policy in prod |
| No memory_limiter ordering issue | 12-16, 44-57 | Correct: `memory_limiter` first, then `batch` |

Pipelines are otherwise structurally sound: logs go to Loki's native OTLP
endpoint (`otlphttp/loki` → `/otlp/v1/logs`, correct for Loki 3.x), traces to
Jaeger, metrics to Prometheus.

---

## 6. What to add for the planned outbox projector

Design source: `docs/plan/DECISIONS.md:15-21`, `docs/plan/00-foundations.md:8-9`,
`docs/plan/01-trends.md:3`, `docs/plan/05-hardening.md:4`.

**Metrics**

| Metric | Type | Labels | Purpose |
|---|---|---|---|
| `outbox_lag_seconds` | gauge | `aggregate_type` | `now() - min(created_at)` of unprocessed rows → primary SLO |
| `outbox_pending_rows` | gauge | `aggregate_type` | backlog depth |
| `outbox_dead_rows` | gauge | `aggregate_type` | rows with `attempts >= cap` |
| `outbox_processed_total` | counter | `aggregate_type`, `outcome` | throughput / success rate |
| `outbox_projector_errors_total` | counter | `kind` | claim/parse/project failures |
| `outbox_retries_total` | counter | `aggregate_type` | churn |
| `outbox_coalesced_rows_total` | counter | — | coalescing efficiency |
| `outbox_batch_duration_ms` | histogram | — | projector loop latency |
| `outbox_batch_size` | histogram | — | batch utilization |
| `outbox_last_success_timestamp_seconds` | gauge | — | staleness alert |
| `outbox_enqueue_total` | counter | `aggregate_type` | written in the same tx as mutations |

**Traces**

- Span `outbox.project_batch` (`batch_size`, `aggregate_type`) with a child span
  per aggregate/row.
- Store the producer `trace_id` (or full `traceparent`) on the outbox row so the
  projector can link projection back to the originating request. Without this,
  the async path is a disconnected trace.
- Spans for `outbox.claim` (`db.system`, `db.operation`, `SKIP LOCKED`), `rebuild`,
  and `enqueue_job`.

**Logs**

- Structured `info!` on claim/process; `warn!` on retry (`outbox_id`,
  `aggregate_id`, `attempts`, `last_error`); `error!` on dead-letter.
- Never log the full changes JSON if it can contain PII; log `aggregate_type` +
  version instead (`PurchaseChanges` at `src/application/purchase/diff.rs:14-20`
  contains store/description/notes and item names).

**Alerts**

```promql
outbox_lag_seconds > 300                       # backlog aging
outbox_dead_rows > 0                           # needs human / replay
rate(outbox_processed_total[5m]) == 0
  and outbox_pending_rows > 0                  # projector stalled
increase(outbox_projector_errors_total[5m]) > 5
```

Also add the projector as its own OTel service (or scope) with
`service.name`, and make sure it uses the propagator from P1-3.

---

## 7. What to add for the planned Python sidecar

Design source: `docs/plan/03-python-sidecar.md`, `docs/plan/DECISIONS.md:7-8`.

**Resource / pipeline**

- `service.name=analytics-sidecar`, `service.version`,
  `deployment.environment`; OTLP endpoint from env; same collector.
- Enable auto-instrumentation for FastAPI/Starlette, `httpx`, `SQLAlchemy`/
  `psycopg`, so HTTP + DB spans are created without bespoke code.

**Trace propagation (cross-language)**

- Both sides must use the W3C `TraceContextPropagator` (P1-3).
- Rust `HttpAnalyticsClient` (plan 3.3) injects `traceparent`; sidecar extracts
  it. Without this the sync inference call appears as an orphan trace.
- Add retries/timeouts/circuit-breaker metrics (plan mentions breaker/fallback):
  `sidecar_client_errors_total{kind}`, `sidecar_circuit_state{state}`.

**Metrics**

| Metric | Type | Labels |
|---|---|---|
| `sidecar_request_duration_ms` | histogram | `endpoint`, `status` |
| `sidecar_errors_total` | counter | `endpoint`, `error_type` |
| `sidecar_up` | gauge | — (or rely on scrape `up`) |
| `job_queue_lag_seconds` | gauge | `job_type` |
| `job_processed_total` | counter | `job_type`, `outcome` |
| `job_failures_total` | counter | `job_type`, `error_type` |
| `inference_duration_ms` | histogram | `model`, `model_version` |
| `model_version_info` | gauge | `model`, `version` (always 1) |
| `training_duration_ms` | histogram | `job_type` |
| `model_promotion_total` | counter | `model`, `result` |
| `db_query_duration_ms` | histogram | `operation` (read-only role) |

**Logs**

- JSON, with `trace_id`/`span_id` for correlation; no PII (no descriptions,
  merchant text, or amounts beyond what's needed). Log model version and
  confidence, not feature vectors.

**Alerts**

- sidecar P95 latency > threshold; error ratio; `job_queue_lag_seconds`;
  job dead-letters; model staleness (`time() - model_loaded_timestamp_seconds`);
  circuit breaker open; sidecar scrape `up == 0`.

**Compose**

- Add the sidecar to `otel-net` with `OTEL_EXPORTER_OTLP_ENDPOINT`, a
  read-only DB role/DATABASE_URL, healthcheck; add `alertmanager`; add volumes
  for Prometheus/Loki/Jaeger; add a Prometheus scrape job if the sidecar exposes
  `/metrics` directly.

---

## 8. Suggested remediation order

1. **P0-2** log/metric internal errors, **P0-3** Alertmanager — restores basic
   incident response.
2. **P0-1 + P1-8** fix metric names and resource→label conversion — restores
   alert fidelity.
3. **P1-1/P1-2/P2-3** fix span args, add `err`, make the 5xx counter monotonic —
   safe traces and correct span status.
4. **P1-7** console log layer + `RUST_LOG` — local operability.
5. **P1-4/P1-5/P1-6/P2-9** add DB/pool/FX/auth/conflict instrumentation.
6. **P1-3/P1-9/P2-1** propagation + route-aware RED metrics.
7. **P2-2/P2-6/P2-8** sampling, collector cleanup, stack hardening.
8. Outbox projector + sidecar instrumentation (§6, §7) as those features land
   (`docs/plan/05-hardening.md:3-4`).

---

## 9. Verification checklist

- `curl -s http://localhost:8889/metrics | grep finmgmt` — list real series and
  reconcile with `alerts.yml`.
- `promtool check rules prometheus/alerts.yml` and `promtool check config
  prometheus/prometheus.yml`.
- Trigger a DB error and confirm an `internal_errors_total` increment and an
  `error`-level log with the cause.
- `docker logs app` should show JSON logs after P1-7.
- Send a request with a known `traceparent` and confirm Jaeger shows it as the
  parent (after P1-3).
- Inspect one `purchase.create` span in Jaeger and confirm no store/notes/amount
  attributes (after P1-1).
- Confirm `finmgmt_db_pool_used`/`_size` and `finmgmt_fx_provider_*` exist
  (after P1-4/P1-5).
