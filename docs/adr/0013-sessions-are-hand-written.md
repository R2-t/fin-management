# A Sessions are ours to write, not a library's to lend

The API needs a cookie session so a person can log in and stay logged in. The
obvious move is `tower-sessions` with a Postgres store, or `axum-login` on top of it,
which is what the ticket first proposed. We looked, and declined.

The reason is version coupling, and it is measurable rather than aesthetic. This
service is on axum 0.7.9. `tower-sessions` 0.14 and 0.15 target axum 0.8: their
`Session` extractor does not satisfy axum 0.7's `Handler` bound, which we confirmed by
compiling a scratch crate against both. `tower-sessions` 0.13.0 does compile on our
line, so the library is available — but it is the last version that will be, and every
future feature of the library assumes an axum upgrade we have not scheduled. Taking it
means the session layer moves when the framework moves, decided by someone else's
release cadence, in an area where a wrong version is a security bug.

What we get in exchange is small. The list of things `tower-sessions` would give us —
session rotation on login, a store trait, expiry — is roughly: a fresh random token on
login, a `DELETE` on logout, and `WHERE expires_at > NOW()`. The cookie is
`HttpOnly` and `SameSite=Strict` either way. Revocation is a row delete either way. The
library's real value is the parts we would have to get right and cannot easily test —
and those parts are short enough that we can test them directly: the token is hashed
with sha256 before it touches the database, so a reader of the table cannot mint a
session; the lookup compares hashes inside Postgres, so no secret byte is compared in
application code where a timing oracle could live; login never reuses a cookie value,
which is the session-fixation fix.

So the session is about 150 lines in `src/auth/session.rs`, one table, and no
dependency. It is unaffected by an axum upgrade. An agent that has to change session
semantics in six months opens one file and reads all of it, instead of learning a trait
boundary and a version pin.

The trade we accepted: a library reviewed by many users versus code reviewed by us. We
pay for that with tests, not with confidence — the lifecycle, the expiry, the rotation
and the revocation each have one, against a real database.

The same reasoning does not apply to password hashing or to PASETO. `argon2` and
`pasetors` implement algorithms where a subtle mistake is silent and total; those stay
in libraries, and this ADR does not argue otherwise.
