# Adversarial review — observability changes

Reviewer: `opencode-go/qwen3.8-flash` (Pi agent), run as seven small read-only
tasks after a single large prompt failed to complete. Only
`docs/reports/qwen-observability-review.md` was written; no code was changed.

Scope: commits `d04f3be..HEAD`, focus on commit `f9367d7`, plus
`Cargo.toml`, `otel-collector-config.yaml`, `prometheus/*`, `Local.DockerCompose.yaml`.

## Verdicts on the author's claims

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Metrics were never exported because `MetricsLayer::new` takes the provider by value and drops it | **HOLDS** | `tracing-opentelemetry-0.30.0/src/metrics.rs:394` takes `meter_provider` by value, calls `meter_with_scope`, drops it; `MetricsLayer` (line 385) keeps only the `Meter`. `opentelemetry_sdk-0.29.0/src/metrics/meter_provider.rs:146-160` shuts the pipelines down on the last `SdkMeterProviderInner` drop, and `SdkMeter` holds only `Arc<Pipelines>`. The fix is correct and matches the crate's own doc example. Caveat: metrics now never shut down cleanly. |
| 2 | Every metric event carries bounded labels only | **HOLDS** (one caveat) | All 12 metric-bearing events use only literals, `&'static str`, validated currency codes, route templates or status codes. Free text is always on a *separate* event (`error=%e`, `panic=%detail`). Caveat: `src/application/purchase/service.rs:90` labels the purchase counter with `rate_source`, a provider-supplied `String`; bounded by convention, not by type. |
| 3 | 5xx marks the request span `ERROR` | **HOLDS** | `otel.status_code = field::Empty` is declared at `src/main.rs:157`, and `tracing-opentelemetry` routes `record_str(SPAN_STATUS_CODE_FIELD)` through `str_to_status` (`layer.rs:444`, `:116`) into the real span status, not just an attribute. Minor gap: no `otel.status_message`. |
| 4 | Inbound W3C `traceparent` is extracted | **HOLDS** | `set_text_map_propagator` runs in `config::init()` before the router is served; `HeaderExtractor` is correct (`HeaderMap::get` is case-insensitive); `set_parent` is applied at `src/main.rs:163`; handler/service spans inherit it. Invalid or absent headers safely produce a fresh root. |
| 5 | Alerts reference series that exist | **PARTLY WRONG** | Names and labels check out, but one rule can never fire and three can flap (see P1-1, P2-1, P2-2). |
| 6 | Signal hygiene (no OTLP log feedback, no ERROR on 4xx, projector safe) | **HOLDS** | EnvFilter silences the exporter crates; 5xx guards are `>= 500` only; the projector sleeps on idle and holds no connection across the sleep. Two gaps at P2-3, P2-4. |

## P1 — actionable

### P1-1 `NoPurchasesCreated` can never fire
`prometheus/alerts.yml:99` — `sum(increase(finmgmt_purchase_created_total[1h])) == 0`.
`finmgmt_purchase_created_total` is created only when the first purchase is
recorded, and `increase()` over a series with no samples yields **no result**,
not `0`. The exact scenario the rule targets (a service that records nothing) is
invisible to it.
**Fix:** `absent(finmgmt_purchase_created_total) or sum(increase(...[1h])) == 0`.

### P1-2 Alerts are dropped at the last mile
`prometheus/alertmanager.yml:18` points at `http://localhost:9999/alerts`, a
placeholder. Inside the alertmanager container that is the container itself, so
every fired alert is silently discarded. Routing, grouping and the Prometheus
`alerting:` block are otherwise wired correctly (`prometheus.yml:9`,
`Local.DockerCompose.yaml:71`).
**Fix:** configure a real receiver (Slack/webhook/email).

### P1-3 The collector image is unpinned under a churned option
`Local.DockerCompose.yaml:31` uses `otel/opentelemetry-collector-contrib:latest`
while `otel-collector-config.yaml:29-31` relies on
`resource_to_telemetry_conversion`. The option was verified working on the
0.159.0 image pulled during this session (metrics carried `service_name`), but
this exporter option has churned across contrib releases, and `:latest` can
change without notice. If it is dropped, the collector fails to start, nothing
is exposed on 8889, every rule breaks, and `TelemetryPipelineDown` fires into
the black hole from P1-2.
**Fix:** pin the image (e.g. `:0.159.0`) and assert the config parses in CI.

## P2 — hygiene

- **P2-1** `FxStaleRateFallbacks` uses `for: 0m` (`alerts.yml:68`): one stale
  fallback fires instantly and stays firing up to 15m as the window decays.
  `HandlerPanics` (`:90`) has no `for` at all. Both flap.
- **P2-2** `HighHttp5xxRatio` needs `finmgmt_http_5xx_total`, which is absent
  until the first 5xx. `clamp_min` prevents a divide-by-zero and an empty result
  never fires, so it is safe — but "no 5xx ever" is indistinguishable from "no
  traffic".
- **P2-3** A panic in the pool sampler (`src/config/mod.rs:103-118`) escapes the
  tracing panic hook: the sampler is spawned inside `config::init()`, while the
  hook is installed later at `src/main.rs:107`. Pool metrics would silently stop.
- **P2-4** The `EnvFilter` default silences exporter crates at INFO/DEBUG, but
  their WARN/ERROR records still cross into `OpenTelemetryTracingBridge`
  (`config/mod.rs:55`) — a bounded feedback path when the collector is slow.
- **P2-5** `src/config/mod.rs:79` comments that propagation covers "outbound
  calls", but no client injects context (`grep` finds no `inject_context`).
  Outbound propagation is not implemented; the comment overclaims.

## Doc contradictions

- `src/config/mod.rs:79` vs code: claims outbound trace propagation (P2-5).
- `docs/OBSERVABILITY.md` is accurate on inbound extraction but implies a
  complete propagation story; it should say outbound is pending the sidecar.

## Not verifiable here

- Whether `resource_to_telemetry_conversion` still exists in whatever
  `:latest` resolves to at deploy time (only the pinned 0.159.0 was exercised).
- Real alert delivery, since no receiver exists.
