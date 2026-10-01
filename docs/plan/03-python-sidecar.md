# Phase 3 — Python sidecar (FastAPI, pandas, scikit-learn; Prophet later)
- [ ] 3.1 Repo skeleton, Dockerfile, compose service, OTLP, health, read-only DB role
- [ ] 3.2 Job worker: poll `analytics_job` (SKIP LOCKED), idempotent handlers
- [ ] 3.3 Rust `SpendAnalytics` trait + `HttpAnalyticsClient` (timeout, retry, breaker, fallback)
- [ ] 3.4 Merchant normalization (rules + rapidfuzz vs `merchant_alias`)
- [ ] 3.5 Categorization v1: merchant defaults → TF-IDF + linear model, confidence threshold
- [ ] 3.6 Feedback capture: `classification_feedback`, `purchase_classification`
- [ ] 3.7 Training job: temporal split, `model_registry`, gated promotion, rollback
- [ ] 3.8 Probabilistic forecast: quantile/bootstrap bands, then Prophet if justified
