# Phase 7 — Identity, sessions, and service tokens

Ticket: [#3](https://github.com/R2-t/fin-management/issues/3). Decisions D69–D75 in
`DECISIONS.md` came from a grill with Jev and are not re-opened here.

Today the API is unreachable in any real configuration: `StaticKeyProvider` holds an
empty map, so every PASETO token gets `KeyNotFound` → 401, and the only way in is
`AUTH_SKIP_VERIFY=1`, which is unguarded. There is no `users` table, so `user_id` is
an opaque UUID with no identity behind it. The verifier reads one claim (`user_id`)
and never checks `exp`, `iss` or `aud`, so a token that never expires is valid
forever.

This phase gives the service a real identity, a real session, and a real token — and
turns the sidecar from a trusted neighbour into an authenticated caller.

## Phase A — identity and sessions

### 7.1 `users` table (U26)

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
`user_id` per test (97 `new_user()` call sites), so an FK would force an async seeding
path through every test file for no containment the app does not already get from the
authenticated principal.

`password_hash` is an Argon2id PHC string. A row whose hash is not a parseable PHC
string **cannot log in** — that is how a seeded or disabled user is represented, rather
than a `disabled` boolean nothing reads. The verifier returns `invalid_credentials`,
never an error that distinguishes "no such user" from "this user cannot log in"
(`VerifyError::InvalidCredentials`), so login does not leak which emails have accounts.

Argon2id parameters: `m_cost = 19456` KiB, `t_cost = 2`, `p_cost = 1` — the OWASP
baseline. Written down here because a future reader will ask, and written as constants
so a test can assert them.

### 7.2 `sessions` table (U27)

```sql
CREATE TABLE IF NOT EXISTS sessions (
    id_hash      BYTEA PRIMARY KEY,          -- sha256(cookie token), never the token
    user_id      UUID NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at   TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions (expires_at);
```

The cookie carries 32 random bytes (base64url). The row stores `sha256(token)`, so a
database reader cannot mint sessions. Lookup is by `id_hash` equality — one index hit —
and the token comparison is over the *hash*, which the database compares bytewise; the
application never compares the raw token, so there is no timing oracle on secret bytes.

Absolute expiry, 30 days, no sliding. A sliding window rewrites a row on every request
and makes "when does this session end" unanswerable; 30 days absolute is a number the
user can be told.

Expired rows are deleted lazily on lookup (the same query that fails to find a live row
deletes it), plus a maintenance sweep. No scheduler: the sweep runs in
`drain_projection`'s sibling maintenance command.

### 7.3 Session auth module (`src/auth/session.rs`)

One file, ~150 lines, and every line is load-bearing:

- `issue(conn, user_id) -> (token, expires_at)` — INSERT with a fresh random token.
- `resolve(conn, token) -> Option<user_id>` — sha256 the token, SELECT … WHERE
  `id_hash = $1 AND expires_at > NOW()`. Bumps `last_seen_at`. Deletes the row if
  expired and returns `None`.
- `revoke(conn, token)` — DELETE by hash. Used by logout.
- `rotate_on_login` is not a separate function: login issues a *new* row and (if the
  request carried a cookie) deletes the old one. A login never reuses a cookie value,
  which is the session-fixation fix.
- Cookie: `HttpOnly`, `SameSite=Strict`, `Path=/`, `Secure` when the request arrived
  over TLS or `APP_SECURE_COOKIES=1`; name `fm_session`, or `__Host-fm_session` when
  `Secure` (the prefix requires `Secure`, `Path=/`, and no `Domain`).

Never logged: the token, the hash, the password, the email in span fields.

### 7.4 Auth endpoints (`src/api/auth/`)

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| POST | `/auth/register` | public | `{email, password}` | 201 `{user_id, email}` + cookie |
| POST | `/auth/login` | public | `{email, password}` | 200 `{user_id, email}` + cookie |
| POST | `/auth/logout` | session | — | 204, cookie cleared |
| GET | `/auth/me` | session or bearer | — | 200 `{user_id, email}` |

Register logs the user in (one round trip; a registration that requires a second request
to be useful is a step the user did not ask for). Duplicate email → 409. Password rules:
minimum 12 characters, no maximum below 128, no composition rules — length is the only
strength signal worth enforcing.

`/auth/register`, `/auth/login` and `/health` are registered **outside** the
`auth_middleware` layer. Registering a route before `.layer(from_fn(auth_middleware))`
is public; after it is protected. The router already relies on this for `/health`.

`/auth/me` returns the email; a session principal carries only `user_id`, so it reads
the row. A bearer principal reads the same row; if the user has no row, `/auth/me`
still returns the `user_id` with `email: null` — a machine token for a fabricated user
is legitimate in tests and must not 500.

### 7.5 Middleware: two paths, one principal

`auth_middleware` resolves in this order, and the order is the security property:

1. **Session cookie.** If `fm_session` is present, resolve it. A present-but-invalid
   cookie is **not** a fallthrough to bearer — it is a 401 with reason
   `session_invalid`. Falling through would let a stale cookie silently become an
   anonymous request, and would let a bearer token rescue a revoked session.
2. **Bearer token.** Only when no cookie was sent. Unchanged `TokenVerifier` path.
3. **Neither.** 401, reason `no_session`.

Both paths produce the same `AuthPrincipal { user_id }`, so no handler changes.

CSRF: `SameSite=Strict` covers the cross-site case. Cookie-authenticated state-changing
requests (anything but GET/HEAD) additionally require that, if an `Origin` header is
present, it matches `APP_ORIGIN`; a mismatch is 403 with reason `csrf`. A missing
`Origin` is allowed because non-browser clients omit it, and a browser always sends it
on a cross-site POST.

### 7.6 Dev bypass guard (D71)

`AUTH_SKIP_VERIFY=1` in a build **without** `cfg!(debug_assertions)` refuses to start:
`build_token_verifier` returns an error the server propagates, with a message naming the
variable and the build. In a debug build it works as today and logs a warning at
`WARN` on every start.

`build_token_verifier()` becomes fallible, which is the API change the acceptance
criterion forces; every caller is a `?` away.

### 7.7 PASETO hardening (D74)

`PasetoVerifier` validates, in addition to the signature and `kid`:

- `exp` — **required**. A token with no `exp` is `InvalidToken`. This is the bug the
  review named.
- `aud` — required, must equal `PASETO_AUDIENCE` (default `financial-management`).
- `iss` — required, must equal `PASETO_ISSUER` (default `financial-management`).
- `sub`/`user_id` — `user_id` required, UUID.

`ClaimsValidationRules` from `pasetors` is configured explicitly rather than
`::new()`, so the rules are visible in the code.

Key provider: `StaticKeyProvider` is replaced by `EnvKeyProvider`, which reads
`PASETO_KEY_<KID>` (base64) and `PASETO_ACTIVE_KID`. Startup fails with a named error
when neither is set, instead of every request returning 503. `StaticKeyProvider` stays
for tests.

### 7.8 Rate limiting login (D73)

`login_attempt(id, email, client_ip, attempted_at, succeeded)` — one INSERT per attempt,
success or failure. Before verifying a password, count failures for that email in the
last 15 minutes; ≥ 5 → 429 with reason `rate_limited`. Same for the client IP at ≥ 20.
The IP is read from `X-Forwarded-For`'s first entry **only** when `TRUST_PROXY=1`;
otherwise the socket address. Trusting the header by default would let anyone bypass the
limit by sending a fresh header each time.

Cost: one row per attempt. A successful login deletes that email's failure rows.

## Phase B — service tokens (D68, D72)

### 7.9 Service token format

PASETO v4.local, symmetric, the same key provider and `kid`-in-footer scheme as bearer
tokens, so there is one key story in the deployment.

Claims: `{ iss, aud, sub: "service:api", exp, iat, user_id, scope: ["analytics"] }`.
`exp` is minted at **60 seconds** for HTTP calls and at **300 seconds** for job rows
(D72): long enough that a queued job claimed immediately runs, short enough that a
compromised sidecar holding a job token cannot act for long.

Minting lives in `src/auth/service_token.rs`, behind a trait
`ServiceTokenMinter { fn mint(&self, user_id, scope, ttl) -> Result<String> }` so tests
can mint with a fixed clock and TTL.

### 7.10 The sidecar verifies, and takes the tenant from the claim

`HttpAnalyticsClient` sends `Authorization: Bearer <token>` on every call, minted per
request for the tenant being served. The sidecar's `/normalize`, `/match` and
`/runway-bands` gain a dependency on a `verify_service_token(token) -> (user_id, scope)`
that:

- verifies signature, `kid`, `exp`, `aud`, `iss`;
- **rejects a request whose body or query names a different `user_id`** than the claim.
  This is the point: not "the sidecar checks a token" but "the token decides the
  tenant", so a compromised caller with a valid token for tenant A cannot ask about B.
- sets `app.user_id` from the claim, in the same transaction as the work (the bug found
  and fixed in `6eb93f9`: a `set_config` in a transaction that then commits is gone).

Python side: `pyseto` (already a small, audited library). The verification lives in one
module, `sidecar/app/service_auth.py`, used by both the HTTP app and the worker.

### 7.11 Job rows carry a token (D72)

`analytics_job` gains `auth_token TEXT` (U29). Rust writes it at enqueue time. The
worker:

- takes `user_id` from the verified claim, never from `payload`;
- treats a missing, expired, or bad token as a **terminal failure** with
  `last_error = 'token_expired'` (no retry, no unscoped run). The purchase can be
  re-classified on its next edit; a job that runs as the wrong tenant cannot be undone.

### 7.12 Fallback stays honest

`NoAnalytics` remains the fallback when the sidecar is unreachable, and the response
says the suggestion is unavailable rather than inventing one (the app's standing rule).
A sidecar that returns 401 because of a bad token is *not* a silent fallback: it is
logged at `WARN` with reason `service_token_rejected`, because that is a configuration
fault the operator must see, not a transient outage.

## Tests

- **Session lifecycle** (`tests/auth.rs`, real DB, in-process router): register → login →
  authenticated request → logout → the same cookie is rejected. Session expiry (insert a
  row with `expires_at` in the past). Two sessions for one user; logout of one leaves the
  other working. A login with a pre-existing cookie issues a different cookie value.
- **Tenancy** (`tests/auth.rs`): two registered users cannot see each other's wallets,
  under the session path *and* the bearer path.
- **Hardening** (`tests/auth.rs`): a PASETO token with no `exp` is 401; wrong `aud` is
  401; unknown `kid` is 401; `AUTH_SKIP_VERIFY=1` is refused when built without
  `debug_assertions` (asserted against `build_token_verifier` returning `Err`, since the
  test binary is a debug build).
- **Rate limit** (`tests/auth.rs`): six failed logins → 429; a successful login resets.
- **Unit** (`src/auth/session_test.rs`): token generation is 32 bytes of entropy, hashes
  are stable, an expired row is deleted on lookup.
- **Service tokens** (`tests/service_token.rs`): a minted token verifies; a scope
  mismatch is rejected; the sidecar rejects a body naming another tenant (Python test in
  `sidecar/tests/test_service_auth.py`, no DB needed for the forgery cases).

Every test that needs the database fails loudly without `TEST_DATABASE_URL`, per the
existing harness convention. No test skips silently.

## Out of scope

Email verification, password reset, MFA, OAuth, session device listing, and any axum
0.8 upgrade. Each is its own ticket; the last one would be forced on us by
`tower-sessions` 0.14+, which is exactly why D69 declined it.
