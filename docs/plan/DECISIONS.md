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

## Round 5 decisions (Phase 3 sidecar) — all four re-asked with concrete cases after the abstract phrasing split

| # | Decision | Outcome | Source |
|---|---|---|---|
| D46 | Merchant alias match threshold | **85** on the similarity ratio. Near-identical strings link; a descriptor that adds a real word (`TARGET PHARMACY` vs `TARGET`) stays separate, because the user budgets groceries and medicine apart | Jev 0.88 (conf 0.82) on the concrete case, after an abstract ask split 0.50/0.47 |
| D47 | Category auto-assign threshold | **0.75** confidence. Below it the purchase stays uncategorized and the user is asked | Jev 0.93 (conf 0.91) |
| D48 | Model promotion gate | **The candidate must beat the active model on every reported metric.** A model that is better overall and worse on every class does not ship | Jev 0.91 (conf 0.86) on the concrete case, after an abstract ask split 0.60/0.31 |
| D49 | Forecast uncertainty | **Bootstrap resampling of the user's own daily spending.** No new model class, uses only data the app already has, and the bands describe her real variability. Prophet stays out (D8) | Jev **1.00** on the concrete case, after an abstract ask split 0.60/0.40 |

### Note on the abstract phrasing

Three of the four asked abstractly sat near 0.5 and split across plausible options. Re-asked with
Maria and named cases (`NETFLIX.COM` vs `NETFLIX`, `TARGET PHARMACY` vs `TARGET`, a model that is
better overall and worse on every class) they resolved to 0.82, 0.86 and 1.00. This is the third
time in this repo that phrasing moved the answer more than confidence did — the same lesson as the
currency-exponent decision.

## Round 6 decisions (wallet currency change) — found by review, not by the plan

A second agent reviewing the Phase 2/3 work found that a wallet's currency could be changed after
it had ledger entries, and that the reported balance then summed two currencies under one label.
Verified against the running service: a wallet holding 1000 USD, changed to EUR, returned
`{balance_minor: 1000, currency: "EUR"}`. A thousand dollars reported as a thousand euros.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D50 | Currency change on a wallet that holds entries | **Rejected.** "This wallet holds a balance in USD; create a new wallet instead." The one-currency-per-wallet invariant is what keeps the balance, the runway and every budget reading a single number | Jev **1.00** once the cascade was stated (runway, overspend forecast and every budget all need one number); 0.58 vs 0.38 split when it was not |
| D51 | Currency change on an empty wallet | **Disables the budgets, income schedules and recurring rules pointed at it.** All three are denominated in the old currency | Jev 0.84 (conf 0.76) |
| D52 | Glossary definition of a Wallet | **Unchanged.** "A single pot of money the user holds, in one currency" stands as an invariant the system enforces, not as a description of what it tolerates | Jev noul **0.14** for "the definition must change" |

The path a user actually takes: create a wallet in the new currency and record an opening balance
in the amount she really has — a number she enters herself, which is not an invented one. The old
wallet keeps its entries as history.

This is the fourth time phrasing moved the answer: asked without the cascade (balance, runway,
overspend and budgets all needing one number) the balance question split 0.58/0.38 at confidence
0.44. Stated with it, 1.00.

## Round 7 decisions (dead budget columns) — both resolved on the first ask

The review found two columns that were written, stored and returned to the API caller while no
code path acted on them. Both were worse than absent, because each invited the reader to expect an
effect.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D53 | `budget.rollover_unspent` | **Removed** from the schema, the model and the API. D30 recorded two independent carry flags; ADR-0010 later settled that unspent money is reported beside the limit and never raises it, which leaves this flag describing behaviour the product has rejected. The API accepted it from the caller and handed it back, so a caller could set it, see it come back, and nothing differ | Jev **0.97** (conf 0.96), first ask |
| D54 | `budget_period.unspent_minor` | **Removed as a column**, kept as a derived figure on the status response. It was written as 0 at materialisation and never updated, while the API computed the real value at read time -- the database said 0 and the API said 200 for the same period. The value changes every day as the user spends, so it cannot be meaningfully written once | Jev **0.94** (conf 0.91), first ask |

Both asked concretely (Maria's 500 budget, 200 left over in March) and both resolved on the first
attempt, unlike Rounds 5 and 6 where the abstract phrasing split. A test now asserts that the
unspent figure *moves* as she spends, which is what distinguishes a derived value from a stored
one.

## Round 8 decisions (income receipt idempotency) — a double-credit that the schema could not prevent

The endpoint that records an income arrival could be called twice -- a double-click, a client retry
after a timeout -- and each call wrote a receipt AND credited the wallets again. Because the credit
carried no reference to the receipt that caused it, a duplicate could not even be found afterwards.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D55 | The occurrence key | **The schedule plus the calendar date the money arrived on**, in the user's timezone. One arrival per schedule per day. Not the exact instant: a retry that omits the timestamp defaults to now and would not match the first call | Jev 0.93 (conf 0.91), first ask |
| D56 | What a repeat returns | **The existing receipt, with a 200 and a `repeat` flag.** She cannot tell a timed-out-but-completed request from a never-completed one, so both must look like success; the flag keeps the double-click visible | Jev 0.83 (conf 0.74) once the two cases were named. Asked abstractly it split 0.58 return / 0.28 conflict, at confidence 0.37 |
| D57 | Credit traceability | **Every credit points back at the receipt that produced it**, so a credit can be traced to its cause and a receipt can be reversed by superseding its entries the way a purchase correction is | Jev **1.00**, first ask |

The uniqueness constraint on `(income_schedule_id, occurred_on)` is what actually enforces D55; the
application reads the existing receipt so the caller gets it back rather than a constraint
violation. The uniqueness rule is the enforcement, not the read.

## Round 9 decisions (unreadable enum values) — no plausible guesses

Five columns hold a fixed set of words, each with a CHECK constraint. When the code read a row
whose word it did not recognise, it substituted a plausible one. The stakes were not cosmetic: a
recurring rule's status read as `active` instead of `cancelled` keeps the forecast charging for a
subscription the user cancelled, and a budget's cadence read as `monthly` puts the period
boundaries on the wrong days.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D58 | An unreadable fixed word | **Stop the request.** A value that violates a CHECK constraint is an impossible state, and the impossible state stops the thread rather than continuing with a guess. The panic is caught by the existing handler: logged with the column and the value, counted as `panics_total`, and returned as a 500 | Jev 0.73 for "fail with an error" on the first ask; re-asked against what an error and a panic actually do here, **0.75 for a panic** (error 0.11). Substituting a default scored **0.00** |

A panic rather than an error because it is a data condition the schema says is impossible, not a
programming bug: the existing panic counter and alert rule then show a schema change that breaks
every read as a spike, rather than as background noise among ordinary database errors. The
alternative -- keeping the value as an `unknown` enum variant -- scored 0.01: every reader would
have to decide what an unknown kind means, and most of them cannot.

## Round 10 decisions (the pace's division) — MONEY.md's rule, applied where it was missing

`MONEY.md` says one rounding rule: half away from zero. The SQL side uses `round()` on NUMERIC,
which does exactly that, and `convert_minor` did too. The burn rate's daily pace divided with a
plain `/`, which truncates toward zero: a pace of 10505 over 7 days read as 1500 instead of 1501.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D59 | The pace's division | **Half away from zero**, through one shared helper that `convert_minor` now uses as well. The rule lives in one place on the Rust side and in `round()` on the SQL side | User decision, following MONEY.md rather than Jev -- this is a convention already recorded, not a new one |

A convention that was recorded twice -- once as a document and once as code -- and implemented a
third time with a different rule, is the kind of drift the plan exists to stop.

## Round 11 decisions (row-level security) — the tenancy boundary becomes the database's

D4 says the sidecar gets a "user-scoped DB role". What existed was one global role whose scoping
lived in the queries it happened to send: a forgotten WHERE clause was not a bug but a leak, and
nothing in the database would have noticed. The sidecar's endpoint takes a `user_id` and trusts it.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D60 | Row-level security on what the sidecar reads | **Enabled** on `merchant`, `merchant_alias`, `purchase`, `purchase_fact` and `category`, with a policy keyed on `set_config('app.user_id', ...)`. The sidecar states which user it acts for and can then see only that user's rows; a connection that states none sees nothing. The application is unaffected -- it connects as the tables' owner, and an owner bypasses row-level security unless it is forced. System categories stay visible to every tenant | User decision, following D4 rather than Jev -- the decision was already recorded; this is enforcing it |

The policy on `merchant_alias` has no `user_id` of its own: it passes through the merchant's
policy, so the alias is visible exactly when its merchant is. Verified with four tests that connect
AS the sidecar's role -- which the rest of the suite never does, because it connects as the owner
and would bypass the boundary it is testing.

## Round 12 decisions (the projection's today boundary) — a charge generated and silently dropped

The review found that the runway generated recurring charges and scheduled income for the range from
TODAY onward, while the simulation consumed only TOMORROW onward. An occurrence landing on today was
generated, stored in a map, and never read: dead data, and a double-count waiting for the loop to
change.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D61 | A charge or paycheck due today | **Not projected.** Occurrences are generated from tomorrow, matching the simulation window exactly. A charge landing on today may already be in the balance the projection starts from, and the projection cannot tell whether it has been recorded -- so including it risks charging it twice, which is the worst outcome. The assumption list states the boundary | Jev **0.99** (conf 0.98), first ask. Subtracting it from the starting balance scored 0.01, including it as the first projected day 0.00 -- both double-count Case A, where the charge is already in the balance |

Whether the hole this leaves -- Maria's rent goes out tonight, her balance was read this morning, so
the projection says she has 1200 more than she will have tomorrow -- is a bug or a limit, Jev scored
0.34: a limit, stated in the assumptions.

## Round 13 decisions (Phase 3 suggest-and-learn) — three rounds, six decisions

The abstract ask split all six between 0.25 and 0.64. Re-asked with Maria and named cases, four
resolved decisively and two stayed soft enough to record as leans. The clearest reversal: asked
abstractly whether the cold start should suggest anything, the answer was "no, wait for a model"
(0.50); asked with Maria's 40 hand-categorized purchases named, it flipped to "the merchant's own
history IS the cold start" at 0.91.

| # | Decision | Outcome | Source |
|---|---|---|---|
| D62 | A purchase the user categorized | **Never changed.** But the system's disagreement is stored as a prediction record, so the learning loop still sees the example: 'we would have said Groceries' is data, not an applied change | Jev 0.87 (conf 0.80), round 2. Round 1's "never even compute" scored 0.13 once the example was named |
| D63 | What a user's own choices teach | **The final settled value only.** 'Drinks' corrected to 'Eating out' teaches Blue Bottle -> 'Eating out'; the intermediate choice is how she got there, not a second opinion. If she edits again, the label moves with it | Jev 0.76 (conf 0.52), round 3. Every historical value as a separate example would teach the loop that she is inconsistent |
| D64 | The cold start | **The merchant's own history suggests.** If every Blue Bottle purchase Maria recorded is 'Eating out', purchase 41 is suggested that at high confidence -- a rule over her own data, no model needed, threshold still applies. This makes the loop useful from her first month rather than after a training run | Jev **0.91** (conf 0.82), round 3, reversing round 1's "nothing until a model" (0.50) |
| D65 | What makes a classify job idempotent | **Purchase id plus the version it was enqueued at.** A stale job -- one whose version is older than the purchase's current version -- is skipped, because the purchase moved on and a stale suggestion would be wrong | Jev 0.61 (conf 0.42), round 2. Recorded as a lean rather than settled; the version check is implemented regardless, because a stale suggestion is wrong whatever the model says |
| D66 | Who applies a suggestion | **The Rust job worker**, which writes purchase.category_id, flips classification_status from 'unclassified' to 'suggested', and records a purchase_classification row with the confidence and model version. The sidecar's role cannot write to purchase -- the D4 boundary, not an inconvenience | Jev **0.98** (conf 0.97), round 2. Giving the sidecar write access scored 0.00 |
| D67 | When a suggestion is computed | **Inline at create, with a 150ms budget, falling back to a background job when the sidecar is slow or down.** The response Maria is holding carries the suggestion when the sidecar is healthy; recording a purchase never waits longer than the budget | Jev 0.80 (conf 0.60), round 3. Round 1's background-only (0.57) dropped to 0.20 once the ten-second gap was named |

| D68 | What carries the calls between the Rust service and the sidecar | **HTTP/JSON, with PASETO tokens on every service-to-service call.** gRPC was considered and rejected: containment under compromise comes from authorization and least privilege, not the wire protocol; the inline classify call has a 150ms budget and sits at 1-3ms either way; protoc joins CI and two toolchains for it | Jev 0.98 (conf 0.98), asked at the owner's request. PASETO itself scored 0.90 -- without it a compromised sidecar impersonates any tenant by setting app.user_id. Key infrastructure shared with ticket #3 |

| D69 | How the cookie session is built | **Hand-rolled: a `sessions` table, a random 256-bit token in an HttpOnly cookie, sha256 at rest, one index lookup per request, a new token on every login.** No `axum-login`, no `tower-sessions`. The library is not merely an extra dependency: 0.14 and 0.15 target axum 0.8 and their `Session` extractor **does not compile** against our axum 0.7 (proved in a scratch crate), and 0.13 is the last version on our line -- so taking it pins the session code to an axum upgrade date we do not control, for ~150 lines we would otherwise own | Jev 0.96 after the version fact was put in the state (it read 0.49, then 0.72, on the same options while the pin was unknown -- phrasing the constraint moved it, the fifth time that has held) |
| D70 | What `users.id` is | **The opaque `user_id` UUID already used everywhere.** No new identity column, no remapping migration, tenancy code and all 184 tests untouched | Jev 0.92 (0.96 vs 0.04) |
| D71 | What `AUTH_SKIP_VERIFY` becomes | **Refuses to start in a build without `debug_assertions`,** with a message naming the variable and the build. In a debug build it works as today and warns on every start. Ignoring the flag in release was the alternative; a production instance that boots with the wrong auth is worse than one that does not boot | Jev 0.98 (0.99 vs 0.01 for deleting the bypass entirely -- the bypass stays because 184 tests and every local run use it) |
| D72 | How the job worker is trusted with a tenant | **Rust writes a short-lived PASETO token (300s) into the job row; the worker takes `user_id` from the verified claim, never from the payload.** An expired token fails the job terminally rather than running it unscoped -- the purchase is re-classified on its next edit, and a job that ran as the wrong tenant cannot be undone | Jev 0.83 (0.89) after sharpening the lifetime case; 0.79 before, which is the closest call in this round |
| D73 | Login brute-force protection | **A `login_attempt` table in Postgres,** counted per email (5 failures / 15 min) and per client IP (20 / 15 min). Survives restarts, holds across replicas, no new dependency; costs one row per attempt | Jev 0.86 (0.91) |
| D74 | Whether PASETO survives sessions | **Yes: PASETO stays for machine and service tokens; sessions are only for humans.** The offline-verifiable property is what the sidecar call needs -- a session cookie would require the sidecar to query the session table | Jev 1.00 (1.00 vs 0.00) |
| D75 | Whether `user_profile` gains an FK to `users` | **No, not in this ticket.** The 184 integration tests fabricate a fresh random `user_id` per test (97 `new_user()` call sites), so the FK would force an async seeding path through every test file. Integrity already comes from the authenticated principal: every production `user_id` in a write path is resolved from a session or a signed claim | Deviation from the ticket's proposed scope, taken on inspection of the tests. The ticket records it |

The projection works from what has been recorded and says when it read it.

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
