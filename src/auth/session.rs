use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::TryRngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::auth::AuthConfig;

/// How a stored token resolved.
///
/// Three cases, not `Option`: the middleware reports `session_expired` for the
/// second and `session_invalid` for the third, and a user whose session ran out
/// deserves a different message from one whose cookie names nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLookup {
    Active(Uuid),
    Expired,
    Absent,
}

pub struct IssuedSession {
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// 32 random bytes, base64url, no padding: 256 bits of entropy, which is why
/// the stored form needs no stretching.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .expect("the OS entropy source is available");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The stored form. The database only ever sees this, so a reader of the table
/// cannot mint a session, and no secret byte is compared in application code —
/// Postgres compares the hash.
fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub async fn issue(
    pool: &crate::PgPool,
    user_id: Uuid,
    config: &AuthConfig,
) -> Result<IssuedSession, String> {
    let conn = pool.get().await.map_err(|e| e.to_string())?;
    let token = new_token();
    let expires_at = chrono::Utc::now() + chrono::Duration::days(config.session_ttl_days);
    conn.execute(
        "INSERT INTO sessions (id_hash, user_id, expires_at) VALUES ($1, $2, $3)",
        &[&token_hash(&token), &user_id, &expires_at],
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(IssuedSession { token, expires_at })
}

pub async fn resolve(pool: &crate::PgPool, token: &str) -> Result<SessionLookup, String> {
    let conn = pool.get().await.map_err(|e| e.to_string())?;
    let hash = token_hash(token);
    let row = conn
        .query_opt(
            "SELECT user_id, expires_at <= NOW() AS expired FROM sessions WHERE id_hash = $1",
            &[&hash],
        )
        .await
        .map_err(|e| e.to_string())?;

    match row {
        None => Ok(SessionLookup::Absent),
        Some(row) => {
            let expired: bool = row.get("expired");
            if expired {
                // Deleted by the lookup that found it: there is no scheduler in
                // this service, and an expired row has no other reader.
                conn.execute("DELETE FROM sessions WHERE id_hash = $1", &[&hash])
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(SessionLookup::Expired)
            } else {
                Ok(SessionLookup::Active(row.get("user_id")))
            }
        }
    }
}

pub async fn revoke(pool: &crate::PgPool, token: &str) -> Result<(), String> {
    let conn = pool.get().await.map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM sessions WHERE id_hash = $1", &[&token_hash(token)])
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Issues a new session, and revokes the one the request arrived with.
///
/// A login never reuses a cookie value: that is the session-fixation fix, and
/// it is why there is no separate `rotate` function.
pub async fn replace(
    pool: &crate::PgPool,
    user_id: Uuid,
    previous_token: Option<&str>,
    config: &AuthConfig,
) -> Result<IssuedSession, String> {
    if let Some(previous) = previous_token {
        revoke(pool, previous).await?;
    }
    issue(pool, user_id, config).await
}

/// The stored form of a token, exposed so a test can assert a row was deleted
/// rather than merely unreachable.
pub fn hash_for_test(token: &str) -> Vec<u8> {
    token_hash(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_256_bits_of_base64url() {
        let token = new_token();
        assert_eq!(URL_SAFE_NO_PAD.decode(&token).unwrap().len(), 32);
        assert_ne!(token, new_token(), "tokens must not repeat");
    }

    #[test]
    fn the_hash_is_stable_and_is_not_the_token() {
        let token = "a-token";
        assert_eq!(token_hash(token), token_hash(token));
        assert_eq!(token_hash(token).len(), 32);
        assert_ne!(token_hash(token), token.as_bytes());
    }

    #[test]
    fn different_tokens_hash_differently() {
        assert_ne!(token_hash("a"), token_hash("b"));
    }
}
