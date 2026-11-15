# Phase 7 — Identity, sessions, and service tokens

Ticket: [#3](https://github.com/R2-t/fin-management/issues/3). Decisions D69–D76 in
`DECISIONS.md` came from a grill with Jev and a spec review; none is re-opened here.

Today the API is unreachable in any real configuration: `StaticKeyProvider` holds an
empty map, so every PASETO token gets `KeyNotFound` → 401, and the only way in is
`AUTH_SKIP_VERIFY=1`, which is unguarded. There is no `users` table, so `user_id` is an
opaque UUID with no identity behind it. The verifier reads one claim (`user_id`) and
never checks `exp`, `iss` or `aud`, so a token that never expires is valid forever.

This phase gives the service a real identity, a real session, and a real token — and
turns the sidecar from a trusted neighbour into an authenticated caller.

Migrations are **U26–U29, and that is the whole set**:

| # | Contents |
|---|---|
| U26 | `citext` extension, `users` |
| U27 | `sessions` |
| U28 | `login_attempt` |
| U29 | `analytics_job.auth_token` |

None of U26–U29 grants anything to the `analytics` role, and none uses
`GRANT … ON ALL TABLES`. U18's `SELECT ON ALL TABLES` was a snapshot of the tables that
existed then, so `users` and `sessions` are invisible to the sidecar; `tests/analytics_role.rs`
pins that with a test rather than trusting it.

## Configuration

Every number below is configuration with a documented default, not a constant buried in
code. They live in one `AuthConfig` struct parsed once at startup, so the spec has no
magic numbers and an operator can see them in one place.

| Setting | Default | Why this default |
|---|---|---|
| `SESSION_TTL_DAYS` | 30 | Long enough that a phone stays logged in; short enough to be answerable |
| `LOGIN_MAX_FAILURES_PER_EMAIL` | 5 | Five tries covers a typo twice over |
| `LOGIN_MAX_FAILURES_PER_IP` | 20 | A shared address has several people behind it |
| `LOGIN_WINDOW_MINUTES` | 15 | The window the counts are taken over |
| `PASSWORD_MIN_LENGTH` | 12 | Length is the only strength signal worth enforcing |
| `PASETO_AUDIENCE` | `financial-management` | Required at startup; no silent default in release |
| `PASETO_ISSUER` | `financial-management` | Same |
| `PASETO_ACTIVE_KID` | — | **Required.** Startup fails without it |
| `APP_ORIGIN` | — | Required when cookies are enabled; the CSRF anchor |
| `SERVICE_TOKEN_TTL_SECONDS` | 300 (jobs), 60 (HTTP) | D72 |
| `TRUST_PROXY` | off | Off means the socket address is the client address |

## Phase A — identity and sessions

### 7.1 `users` (U26)

```sql
CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY,
    email         CITEXT UNIQUE NOT NULL,
    password_hash TEXT   NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

`users.id` **is** the `user_id` already used by every table (D70): no new identity
column, no data migration, tenancy code untouched.

No FK from `user_profile` (D75). The 184 integration tests fabricate a fresh random
`user_id` per test (97 `new_user()` call sites), and `U7__user_profile.sql` already seeds
profile rows from `wallet`/`purchase` user ids that have no `users` row — an FK would
fail on any non-empty database.

`password_hash` is an Argon2id PHC string, which embeds its own parameters, so raising
the cost later still verifies hashes written today. A row whose hash is not a parseable
PHC string **cannot log in** — that is how a seeded or disabled user is represented,
rather than a `disabled` boolean nothing reads.

Argon2id: `m_cost = 19456` KiB, `t_cost = 2`, `p_cost = 1` (OWASP baseline). 19 MiB per
concurrent login is the memory ceiling; at this app's scale that is the right trade
against a faster hash.

**Login must not reveal whether an email exists, by body or by timing.** When no row
matches, verification runs against a fixed dummy PHC hash and fails — the same work,
the same 401. Without that, "no user" returns in microseconds and "wrong password"
returns after 19 MiB and a few milliseconds, which is a working oracle.

Register returns 409 on a duplicate email. That *is* an existence oracle, and it is
accepted deliberately: the only way to hide it is to accept the registration and send a
mail the user cannot act on yet (email verification is out of scope), which would leave
a worse state. Recorded here so nobody "fixes" it into silence later.

### 7.2 `sessions` (U27)

```sql
CREATE TABLE IF NOT EXISTS sessions (
    id_hash    BYTEA PRIMARY KEY,          -- sha256(cookie token), never the token
    user_id    UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions (expires_at);
```

No `last_seen_at`: it would write a row on every request for a value nothing reads.
No background sweep: expired rows are deleted by the lookup that finds them (below), and
there is no scheduler in this service to host a sweep.

The cookie carries 32 random bytes (base64url). The row stores `sha256(token)`, so a
reader of the table cannot mint sessions. Lookup is one index hit on `id_hash`, and no
secret byte is ever compared in application code — Postgres compares the hash.

Absolute expiry, `SESSION_TTL_DAYS`, no sliding.

### 7.3 Session module (`src/auth/session.rs`)

One file, and every line is load-bearing. Four functions and a three-way resolve:

- `issue(conn, user_id) -> IssuedSession { token, expires_at }` — INSERT with a fresh
  random token.
- `resolve(conn, token) -> SessionLookup` where
  `enum SessionLookup { Active(Uuid), Expired, Absent }`.
  Three cases, not `Option`, because the middleware reports `session_expired` for the
  second and `no_session` for the third, and an expired session must not be reported as
  "you sent nothing".
  Implementation is two statements in one transaction, not one clever one: `SELECT
  user_id, expires_at … WHERE id_hash = $1`; if the row exists and `expires_at <=
  NOW()`, `DELETE` it and return `Expired`.
- `revoke(conn, token)` — DELETE by hash, used by logout.
- No `rotate` function: login issues a new row and deletes any row the request's cookie
  named. A login never reuses a cookie value, which is the fixation fix.

Cookie: `HttpOnly`, `SameSite=Strict`, `Path=/`, `Secure` when `APP_ORIGIN` is `https`
or `APP_SECURE_COOKIES=1`; name `fm_session`, or `__Host-fm_session` when `Secure`
(that prefix requires `Secure`, `Path=/`, and no `Domain`). **The middleware resolves
both names, `__Host-fm_session` first**, so a change of scheme cannot make an existing
cookie invisible and fall through to the bearer path.

Never logged: the token, the hash, the password, the email.

### 7.4 Auth endpoints (`src/api/auth/`)

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| POST | `/auth/register` | public | `{email, password}` | 201 `{user_id, email}` + cookie |
| POST | `/auth/login` | public | `{email, password}` | 200 `{user_id, email}` + cookie |
| POST | `/auth/logout` | session | — | 204, cookie cleared |
| GET | `/auth/me` | session or bearer | — | 200 `{user_id, email}` |

**Router order.** `.layer(from_fn(auth_middleware))` wraps every route registered
*before* it, so a route added *after* the layer is public. `src/api/mod.rs` already
relies on this to keep `/health` open. Therefore: `register` and `login` are added
**after** the layer; `logout` and `me` are added **before** it with the other protected
routes. Getting this backwards makes login unreachable or logout unauthenticated, so
the order is a test, not a comment.

**Login errors are their own type.** `LoginError` in `src/api/auth/` carries
`InvalidCredentials`, `RateLimited`, `WeakPassword`, `EmailTaken`, `Csrf`. It is not a
variant of `VerifyError`: that enum is matched exhaustively by `auth_middleware`
(`src/auth/mod.rs`) against a status table that a credentials failure should never
reach.

Register logs the user in. Password rules: `PASSWORD_MIN_LENGTH` characters, no maximum
below 128, no composition rules.

`/auth/me` returns the email. A bearer principal may have no `users` row (a machine
token for a fabricated test user is legitimate), so `email` is `null` rather than a 500.

**CSRF applies to login and register too**, not only to authenticated writes.
`SameSite=Strict` stops a cross-site POST from *reading* the response but not from
*setting* a cookie: a login-CSRF attack that signs the victim into the attacker's
account puts the victim's subsequent writes in the attacker's ledger. So register and
login also check `Origin` against `APP_ORIGIN` when the header is present, and fail 403
with reason `csrf` on a mismatch. A missing `Origin` is allowed (non-browser clients
omit it; browsers always send it on a cross-site POST).

### 7.5 Middleware: two paths, one principal

`auth_middleware` resolves in this order, and the order is the security property:

1. **Session cookie.** `__Host-fm_session` then `fm_session`. If either is present it is
   resolved, and the outcome decides: `Active` → principal; `Expired` → 401 `session_expired`;
   `Absent` (a cookie naming no row) → 401 `session_invalid`.
   **A present-but-bad cookie never falls through to the bearer path.** Falling through
   would let a bearer token rescue a revoked session, and would let a stale cookie
   become an anonymous request.
2. **Bearer token.** Only when no cookie was sent, and only when `Authorization` is
   present. Unchanged `TokenVerifier` path.
3. **Neither.** If `verifier.allows_anonymous()` (the dev bypass), verify as today with
   an empty token; otherwise 401 `no_session`. Without this branch the bypass that 184
   tests and every local run depend on stops working, and `allows_anonymous` becomes
   dead code.

Both paths produce `AuthPrincipal { user_id }`, so no handler changes.

**The pool reaches the middleware as `Extension<PgPool>`**, inserted by `build_router`.
`AppState` does not hold the pool today (it holds the built services), and `build_router`
drops it after `build_app_state`; the auth handlers read it from the same extension.
One carrier, cloned once.

### 7.6 Dev bypass guard (D71)

The guard is a pure function so it can be tested without touching the process
environment, which is global and races across parallel tests:

```rust
fn resolve_bypass(flag: bool, debug_build: bool) -> Result<Option<Uuid>, ConfigError>
```

- `flag && !debug_build` → `Err`, naming the variable and the build. The server refuses
  to start.
- `flag && debug_build` → `Ok(Some(user_id))`, with a `WARN` on every start.
- otherwise → `Ok(None)`, and the PASETO verifier is built.

`build_token_verifier` becomes fallible and is a thin wrapper over this. Tests exercise
`resolve_bypass` for all four inputs; the release branch is asserted directly rather
than by trying to un-debug a debug binary.

### 7.7 PASETO hardening (D74)

`PasetoVerifier` validates, in addition to the signature and `kid`:

- `exp` — **required**. A token with no `exp` is `InvalidToken`. This is the bug the
  review named.
- `aud` — required, equals `PASETO_AUDIENCE`.
- `iss` — required, equals `PASETO_ISSUER`.
- `user_id` — required, a UUID.

`ClaimsValidationRules` is configured explicitly rather than `::new()`, so the rules are
visible in the code.

`StaticKeyProvider` is joined by `EnvKeyProvider`, which reads `PASETO_KEY_<KID>`
(base64) and `PASETO_ACTIVE_KID`, and fails at startup with a named error when they are
missing — instead of every request returning 503. `StaticKeyProvider` stays for tests.

### 7.8 Login rate limiting (D73)

`login_attempt(id, email, client_ip, attempted_at, succeeded)` — one INSERT per attempt,
success or failure.

- **Per IP** (`LOGIN_MAX_FAILURES_PER_IP`) is the hard limit: 429 with reason `rate_limited`.
- **Per email** (`LOGIN_MAX_FAILURES_PER_EMAIL`) is a soft limit: past it, the response is
  429 too, but the window is short and a successful login clears the email's rows. Per-IP
  is the limit that cannot be weaponised, because an attacker who wants to lock the owner
  out must spend addresses; per-email alone would let anyone lock the single owner out of
  their own ledger with five requests.

The client IP is the socket address, or `X-Forwarded-For`'s first entry **only** when
`TRUST_PROXY=1`. Trusting the header by default lets anyone bypass the limit by sending
a fresh value each time.

## Phase B — service tokens (D68, D72, D76)

### 7.9 Service tokens use their own key (D76)

The first draft shared the human bearer key. That was wrong: the symmetric key is enough
to *mint* a token, so a compromised sidecar holding it could forge a principal for any
`user_id` — the exact thing the token exists to prevent. Service tokens therefore use a
**separate key id and secret** (`PASETO_KEY_<SERVICE_KID>`, `PASETO_SERVICE_KID`), and the
sidecar is given only that one. The Rust service holds both.

Claims: `{ iss, aud, sub: "service:api", exp, iat, user_id, scope }`. `exp` is 60 s for
HTTP calls and 300 s for job rows (D72): long enough that a job claimed immediately runs,
short enough that a stolen job token is worth little.

Minting lives in `src/auth/service_token.rs` behind
`ServiceTokenMinter::mint(user_id, scope, ttl)`, so tests can fix the clock and the TTL.
`scope` is one constant, `"analytics"`; it becomes a set when a second scope exists.

### 7.10 The sidecar takes the tenant from the claim

`HttpAnalyticsClient` mints a token per request for the tenant it is serving and sends
`Authorization: Bearer <token>`.

The sidecar's `service_auth.verify(token) -> (user_id, scope)` checks signature, `kid`,
`exp`, `aud`, `iss`, and then:

- **rejects a request whose body or query names a different `user_id`** than the claim.
  This is the point: not "the sidecar checks a token" but "the token decides the tenant",
  so a caller holding a valid token for tenant A cannot ask about B;
- sets `app.user_id` from the claim, in the same transaction as the work (the bug found
  and fixed in `6eb93f9`: a `set_config` in a transaction that then commits is gone).

Python side: `pyseto`, in one module `sidecar/app/service_auth.py`, used by both the HTTP
app and the worker. The sidecar receives only `PASETO_SERVICE_KEY_<KID>` and the service
`kid`; it never receives the human bearer key.

### 7.11 Job rows carry a token (D72)

`analytics_job` gains `auth_token TEXT` (U29). Rust writes it at enqueue time; the worker:

- takes `user_id` from the verified claim, never from `payload`;
- treats a missing, expired, or bad token as a **terminal failure** with
  `last_error = 'token_expired'` — no retry, no unscoped run. **The worker cannot re-mint
  a token**: minting requires the human-side key, which the sidecar does not hold, and
  giving it that key would defeat D76. A delayed job is therefore re-classified on the
  purchase's next edit, which is correct because the merchant history has changed anyway.

### 7.12 Fallback stays honest

`NoAnalytics` remains the fallback when the sidecar is unreachable, and the response says
the suggestion is unavailable rather than inventing one. A sidecar 401 because of a bad
token is **not** a silent fallback: it is logged at `WARN` with reason
`service_token_rejected`, because that is a configuration fault the operator must see,
not a transient outage.

## Tests

- **Session lifecycle** (`tests/auth.rs`, real DB, in-process router): register → login →
  authenticated request → logout → the same cookie is rejected. Expiry (a row with
  `expires_at` in the past → `session_expired`, and the row is gone afterwards). Two
  sessions for one user; logging out one leaves the other working. A login carrying an
  old cookie issues a different cookie value and deletes the old row.
- **Middleware order** (`tests/auth.rs`): `POST /auth/login` succeeds with no cookie and
  no bearer token; `POST /auth/logout` without either is 401. This is the test that fails
  if the router order is reversed.
- **Tenancy**: two registered users cannot see each other's wallets, under the session
  path *and* the bearer path.
- **Hardening**: a token with no `exp` is 401; wrong `aud` is 401; unknown `kid` is 401;
  a token with a valid `kid` and valid claims **verifies** (the positive case, so the
  suite cannot pass by rejecting everything). `resolve_bypass` is unit-tested for all
  four inputs.
- **Login**: six failures from one address → 429; a success clears the email's rows; a
  login for an email that does not exist takes the same code path and returns 401, and a
  unit test asserts the dummy hash is verified.
- **Rate-limit and hashing unit tests** (`src/auth/password_test.rs`,
  `src/auth/session_test.rs`): hash/verify round trip, a wrong password fails, a
  non-PHC hash fails closed; token entropy is 32 bytes, hashes are stable, an expired row
  is deleted on lookup.
- **Observability**: `auth_failures_total` covers `no_session`, `session_expired`,
  `session_invalid`, `invalid_token`, `unknown_key`, `invalid_credentials`, `csrf`,
  `rate_limited`; login/logout/register have counters; no `user_id` or password material
  in any log or span field (asserted by grepping the emitted JSON in one test).
- **Service tokens** (`tests/service_token.rs`, `sidecar/tests/test_service_auth.py`): a
  minted token verifies; a scope mismatch is rejected; a body naming another tenant is
  rejected; the sidecar rejects a token signed with the *human* key id; an expired job
  token fails the job terminally.
- **Role boundary** (`tests/analytics_role.rs`): the `analytics` role cannot select from
  `users` or `sessions`.

Every test that needs the database fails loudly without `TEST_DATABASE_URL`. No test
skips silently.

## Out of scope

Email verification, password reset, MFA, OAuth, session device listing, a session
maintenance sweep, and any axum 0.8 upgrade. The last would be forced by
`tower-sessions` 0.14+, which is exactly why D69 declined it.

## Status

**Phases A and B are built and reviewed.** Login, sessions, the PASETO hardening and
the service-to-service token all work; the suggestion pipeline runs inline, falls back
to the queue, and is applied by a Rust consumer.

Settled after the reviews (D77–D81), each closing a finding:

- **D77** an edit that could change the answer re-queues a suggestion; editing notes
  does not.
- **D78** a 60s sweep re-enqueues purchases whose job credential expired unclaimed —
  the failed-create case, which nothing else repaired.
- **D79** a partial unique index replaces the `WHERE NOT EXISTS` enqueue guard, which
  two concurrent statements both passed.
- **D80** the sidecar holds one small `psycopg_pool` instead of a connection per call.
- **D81** the router takes the analytics backend, so the inline path has a test; it
  was untested before, and the suite passed with the inline call deleted.

Known gaps, still open:

- `analytics_job` has no claimed-at column, so a worker killed mid-job leaves the row
  `running` forever. Recorded as a `shortcut:` in `jobs.py`.
- The suggestion sweep and the applier both run on a timer in `main.rs`; there is no
  command to run them once, which makes them awkward to test end to end.
