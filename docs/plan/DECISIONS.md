# Design Decisions (checked with Jev `jev-latest`, probabilities in brackets)

| # | Decision | Choice |
|---|---|---|
| D1 | Analytics store | Append-only `purchase_fact` hypertable; never hypertable `purchase` (0.97) |
| D2 | Fact population | Transactional outbox + Rust projector, not dual-write (0.96) |
| D3 | Rust↔Python | Sync HTTP for inference + Postgres `analytics_job` queue for batch; no broker (0.99) |
| D4 | Python data access | Read-only, user-scoped DB role; writes only its own tables (0.99) |
| D5 | Forecast split | Deterministic runway/burn in Rust (i64); Python adds probabilistic bands (0.99) |
| D6 | Order | Foundations → trends → budgets/forecast → ML → anomalies (1.0) |
| D7 | Timescale timing | Adopt image early, but CAGGs only when needed (adopt-now 0.20) |
| D8 | Prophet | Not in first forecast release; robust baseline first (0.26) |
| D9 | Tenancy | Multi-user: `user_id` on every table/query; global model + per-user merchant aliases |
| D10 | Timescale timing | **Adopt now** (user decision; Jev 0.54 to defer) |
| D11 | Uncategorized purchases | `purchase.category_id` **nullable** + `classification_status`; Python may override later (user decision, overriding Jev's system-category 0.94) |
| D12 | Reporting currency | `user_profile.base_currency` default, `wallet.currency` override (Jev 0.77) |
| D13 | Exchange rate storage | Scaled BIGINT (`rate * 1e9`), one rounding rule (Jev 0.88) |
| D14 | Phase 0 scope | Single phase, includes `purchase_fact` + projector (user decision; Jev 1.0 for splitting) |

## Round 2 decisions (grilling, Phases 1-3 shape) — supersedes parts of D12/D13

| # | Decision | Outcome | Source |
|---|---|---|---|
| D15 | History restatement | **Never** restate a reported amount when currencies/settings change; filling a previously *missing* conversion is completion, allowed | Jev yes=0.16 that restating is OK → rejected. ADR-0001 |
| D16 | Wallet currency change | Existing facts keep purchase-time base currency; **no** wallet outbox events. Spec finding "(a)1 wallet staleness" is therefore **not a bug** | Jev 0.95 keep_point_in_time |
| D17 | Reporting axis | **Per-wallet only.** Cross-wallet consolidation dropped (reverses earlier Q9 answer), so no third amount column and `purchase_fact` needs no migration | User decision; Jev had said three_amounts 0.71 for the consolidation we abandoned. ADR-0002 |
| D18 | D12 amendment | `user_profile.base_currency` is retained but means "default for new wallets" only — it is **not** a reporting/consolidation axis | User: "leave it, useful for new wallets" |
| D19 | Balance model | Append-only `ledger_entry`; balance = sum of entries. Mutable balance column rejected (Jev 0.01) | Jev 0.85; ADR-0004 |
| D20 | Transfers | **First-class `transfer` object** generating its debit/credit legs; legs never edited directly. Income split uses the same one-object-many-entries shape | User decision over Jev's lean to legs-only (0.44 at conf 0.25); ADR-0003 |
| D21 | Missing rate | **Do not fail the insert.** Record with NULL conversion + unresolved marker, resolve later | Reverses current behaviour, which returns 500 (verified). ADR-0006 |
| D22 | Rate storage | Keep unlogged cache for hot lookups; add a logged durable `fx_rate_history` (hypertable candidate). Provider historical endpoint covers legacy gaps | Jev 0.67 hypertable, UNLOGGED-is-defect 0.68; ADR-0005 |
| D23 | Rate retry mechanism | `analytics_job(kind='resolve_rate')` as durable record + periodic sweep safety net; **Rust worker built in Phase 2** (queue has no consumer today) | Jev 0.71 job+sweep, 0.71 rust-worker-now; matches user's (b)+(d) preference |
| D24 | Rate becomes mutable | Add `exchange_rate_scaled` to `PurchaseUpdates` + UPDATE SQL. Verified blocker: today the rate is written only at INSERT and cannot be changed at all | User decision |
| D25 | Timezone bucketing | Query-time `time_bucket(width, ts, tz)`; do not materialize local dates on facts (a timezone edit would force mass reprojection) | Jev 0.92 |
| D26 | Next build order | Clear **correctness AND structural** debts before Phase 1 (user expanded scope beyond Jev's correctness-only 0.99), incl. DB-backed integration tests (Jev 0.77; defer-risk scored high-severe) | User decision |
| D27 | API rate format | Keep JSON number for `exchange_rate_override`. Measured: 79,996 realistic rate strings parsed via f64 matched decimal parsing with **zero** mismatches; only sub-nano precision is discarded | Empirical verification |

## Round 3 decisions (grilling, Phase 2 budgets) — supersedes part of the D18-era "rollover" idea

| # | Decision | Outcome | Source |
|---|---|---|---|
| D28 | Budget shape | **Template + materialised periods.** Template holds limit/category/wallet/cadence/reset day; each period holds the limit that applies during it. Editing a period does not change the template | User decision; ADR-0009 |
| D29 | Cadence anchor | **Reset day** (1 = calendar month, 15 = mid-month payday, -1 = month end), not a calendar month and not an interval from `starts_on` | User decision; ADR-0009 |
| D30 | Carry flags | Two independent flags: `rollover_unspent` and `carry_overspend`, **both defaulting to true** — the ledger never resets, so the budget's carry is a reporting choice about which limit applies | User decision, reasoning: "a ledger never really resets its values" |
| D31 | Period materialisation | **Lazy, on read.** No scheduler. A uniqueness constraint prevents two concurrent requests creating the same period | User decision; ADR-0009 |
| D32 | Wallet currency change | **Disables the wallet's budgets** and the API says so. Re-denominating would need a rate at change time; inventing one is refused | User decision; ADR-0009 |
| D33 | Deleting a wallet or category | **Rejected** when an active budget references it. One rule for both | User decision (Q7, option d) |
| D34 | Unspent carry vs limit | **Unspent money does NOT raise the next period's limit.** It is surfaced separately as money left over, because the user may be deliberately saving it. Jev scored "raise the limit" at **0.00** on the concrete case | User decision (Q5); Jev 0.82 for separate-savings, 0.00 for raise-limit |
| D35 | Limit changes | **Only the user raises a limit.** A carry may lower it (the overspend is a fact the user created, and lowering restricts rather than expands spending), but nothing automatically raises it | User decision (Q6) |
| D36 | Negative wallet balance | **Independent of the budget carry.** The budget carry is computed from spending against the limit and works identically whether the balance is positive or negative; both facts are shown separately. Jev 1.00 | User question; Jev 1.00 independent, 0.00 each alternative |
| D37 | Editing a period | **Applies to that period only.** The next period is unaffected and the user can edit it too. Jev 0.64 (conf 0.46) — low confidence, user decides | User decision (Q6); Jev 0.64 keep, 0.18 recompute, 0.18 flag |

## Round 4 decisions (grilling, Phase 2 runway) — supersedes the run/forecast shape in analytics-investigation §4.3

| # | Decision | Outcome | Source |
|---|---|---|---|
| D38 | Negative balance | **Project forward and report both the trough and the recovery date.** The balance may already be negative; the useful question is how deep it gets and whether income covers it. Degrades to "recovery date" when the trough is behind the user | User decision (R1) |
| D39 | Simulation start | **Today's ledger balance, simulating from tomorrow.** Today's spending is not projectable without inventing a number. The response carries `as_of` so the caller knows the moment the balance was read | User decision (R2) |
| D40 | What the simulation projects | **Burn rate is discretionary spending only**; recurring charges are separate dated line items. Otherwise every subscription is subtracted twice | User decision (R3) |
| D41 | Recurring matcher | **Built now and shared.** The matcher that decides whether a recorded purchase is a rule's payment is needed by the burn rate now and by Phase 4.4's unintended-recurring detection later | User decision; `application::recurring::matcher` |
| D42 | Burn-rate window | **30 days**, falling back to however much history exists (minimum 7 days). Below 7 days of history, **no burn rate is reported at all** — inventing a pace is refused | User decision (R4) |
| D43 | Daily profile | **The weekday median profile when there is enough history for it, a flat daily average otherwise.** The profile needs roughly a month to be meaningful | User decision (R5) |
| D44 | No income scheduled | **The horizon ends and the answer is "still solvent within N days"**, not "you never recover". The horizon is explicit in the response | User decision (R6) |
| D45 | Horizon | **90 days**, with the recovery date reported alongside | User decision (R7) |

### Notes
- Jev's first exponent-urgency answer was "cosmetic" (conf 0.95) asked abstractly; re-asked with a
  concrete JPY user it became 3.35/4 "blocking". Lesson: phrasing moved it more than confidence did.
- The code-review claim that `--reconcile-facts` is tautological was tested and is **false**: corrupting
  `amount_base_minor` returned exit 1. Both sides are independent (stored copy vs re-derived).

## Consequence of D11
`purchase_items` joins `category` with an inner JOIN, so uncategorized purchases would disappear
from the view. It must become a `LEFT JOIN`, and `category_id` must become `Option<i32>` in the model,
repository row mapping, commands, handler DTOs and `CreatePurchaseCommand`.

## Outbox design (D2)
- `analytics_outbox(id, user_id, aggregate_type, aggregate_id, aggregate_version, created_at, processed_at, attempts, last_error)`; written in the same tx as the OLTP mutation.
- IDs only; projector re-reads current state → idempotent, order-insensitive, coalescing.
- Projector: Rust task, `FOR UPDATE SKIP LOCKED` batches, LISTEN/NOTIFY + poll, per-user batch cap.
- Rebuild = delete by `purchase_id` + insert (partition key includes `occurred_at`).
- Same projector enqueues `analytics_job` rows for Python work.
- Attempts capped → dead state; OTel gauge for lag; backfill command; purge processed after ~7d.
- Responses carry `as_of`; short windows may query OLTP live.
