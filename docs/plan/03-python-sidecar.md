# Phase 3 — Python sidecar (FastAPI, pandas, scikit-learn; Prophet later)
- [~] 3.1 Repo skeleton, Dockerfile, compose service, OTLP, health, read-only DB role
- [ ] 3.2 Job worker: poll `analytics_job` (SKIP LOCKED), idempotent handlers
- [ ] 3.3 Rust `SpendAnalytics` trait + `HttpAnalyticsClient` (timeout, retry, breaker, fallback)
- [~] 3.4 Merchant normalization (rules + rapidfuzz vs `merchant_alias`)
- [ ] 3.5 Categorization v1: merchant defaults → TF-IDF + linear model, confidence threshold
- [ ] 3.6 Feedback capture: `classification_feedback`, `purchase_classification`
- [ ] 3.7 Training job: temporal split, `model_registry`, gated promotion, rollback
- [ ] 3.8 Probabilistic forecast: quantile/bootstrap bands, then Prophet if justified

## Status

**The sidecar exists and serves.** `sidecar/` holds a FastAPI app with a health
endpoint and merchant name cleaning, an image built from the repository root,
and a compose service on the `otel-net` network.

```
GET  /health                 liveness
POST /normalize-merchant     {raw: "..."} -> {normalized, changed, model_version}
```

Run the tests with `make test-sidecar`.

**3.1's read-only DB role is deferred to 3.2**, deliberately: the sidecar does
not touch the database yet, so a role would be a control with nothing to
control. It ships with the first code that connects.

**3.4's rules half is done.** The deterministic transforms are in
`sidecar/app/normalize.py` and covered by tests. The fuzzy match against
`merchant_alias` needs the database, so it lands with 3.2.

## Notes

- **The rules are deliberately conservative.** A prefix that is wrong merges
  two merchants that happen to share a brand, which is worse than leaving
  noise in place. The processor list is short for that reason.
- **`shortcut:`** a merchant whose name ends in four to six digits loses them
  (`7-ELEVEN 2471` → `7-ELEVEN`). Descriptors append store numbers far more
  often than merchants are named after them. Worth it; revisit if a merchant
  is ever reported merged by mistake.
- **A descriptor with no letters is empty**, not kept: it is a phone number or
  a reference code, and keeping it would match every other one to the same
  merchant.
- **Non-ASCII names survive.** The separator filter is per character and uses
  `isalnum()`, so `CAFÉ CENTRAL` is not reduced to `CAF`.
- **Telemetry is traces only, and optional.** Without `OTLP_ENDPOINT` the
  service runs uninstrumented rather than refusing to start: a sidecar that
  cannot reach its collector should still serve. Metrics and log exporters
  land when there is something to measure.
- **The image is built from the repository root** (`-f sidecar/Dockerfile .`)
  so the root `.dockerignore` applies and keeps the 80 MB local virtualenv out
  of the build context.
