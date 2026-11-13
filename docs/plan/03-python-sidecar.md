# Phase 3 — Python sidecar (FastAPI, pandas, scikit-learn; Prophet later)
- [x] 3.1 Repo skeleton, Dockerfile, compose service, OTLP, health, read-only DB role
- [ ] 3.2 Job worker: poll `analytics_job` (SKIP LOCKED), idempotent handlers
- [x] 3.3 Rust `SpendAnalytics` trait + `HttpAnalyticsClient` (timeout, retry, fallback)
- [x] 3.4 Merchant normalization (rules + rapidfuzz vs `merchant_alias`)
- [ ] 3.5 Categorization v1: merchant defaults → TF-IDF + linear model, confidence threshold
- [ ] 3.6 Feedback capture: `classification_feedback`, `purchase_classification`
- [ ] 3.7 Training job: temporal split, `model_registry`, gated promotion, rollback
- [x] 3.8 Probabilistic forecast: bootstrap bands on the runway; Prophet rejected (D49)

## Status

**The sidecar exists and serves.** `sidecar/` holds a FastAPI app with a health
endpoint and merchant name cleaning, an image built from the repository root,
and a compose service on the `otel-net` network.

```
GET  /health                 liveness
POST /normalize-merchant     {raw: "..."} -> {normalized, changed, model_version}
```

Run the tests with `make test-sidecar`.

**3.1 and 3.4 done.** The sidecar has a read-only database role and matches
cleaned names against the user's recorded `merchant_alias` rows.

The role is `analytics`: read-only on the application's data, able to write
only the three tables it owns (D4). Verified: it reads `purchase` and is denied
`UPDATE` on it. `U18` creates the role idempotently, because
`DROP SCHEMA public CASCADE` does not drop roles and a database replayed from
empty would otherwise fail on a role that outlived its schema.

**The match threshold is 85 (D46), and a domain suffix is cleaned rather than
the threshold lowered.** Measured against the cases that set the decision:

| pair | raw score |
|---|---|
| `TARGT` / `TARGET` | 90.9 |
| `NETFLIX.COM` / `NETFLIX` | 77.8 |
| `TARGET PHARMACY` / `TARGET` | 57.1 |
| `STARBUCKS` / `TARGET` | 40.0 |

`NETFLIX.COM` at 77.8 would not link at 85, which contradicts the decision's
own stated intent. Lowering the threshold to ~70 would fix it but lands in the
bucket Jev explicitly rejected. Treating a domain suffix as descriptor noise
-- which is what it is -- keeps 85 meaningful, because that number is
calibrated for names.

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
- **`shortcut:`** the sidecar opens a database connection per request rather
  than holding a pool. Fine at a personal-finance row count; add
  `psycopg_pool` when the classify endpoint is called in a loop.
- **A configured database that fails is a 500, not a silent "no match".** The
  caller must be able to tell "there is no merchant" from "I could not check".
  With no `ANALYTICS_DB_URL` at all, matching is skipped so tests run without
  a database.
- **Telemetry is traces only, and optional.** Without `OTLP_ENDPOINT` the
  service runs uninstrumented rather than refusing to start: a sidecar that
  cannot reach its collector should still serve. Metrics and log exporters
  land when there is something to measure.
- **The image is built from the repository root** (`-f sidecar/Dockerfile .`)
  so the root `.dockerignore` applies and keeps the 80 MB local virtualenv out
  of the build context.
