use thiserror::Error;

use super::AuthPrincipal;

/// Errors that can occur while verifying a token.
///
/// These are intentionally library-agnostic so the middleware and handlers do
/// not depend on any specific token implementation.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// The token is malformed, expired, or fails cryptographic verification.
    #[error("invalid token")]
    InvalidToken,
    /// The token references a key id that could not be resolved.
    #[error("token key not found")]
    KeyNotFound,
    /// The external key service could not be reached.
    #[error("key provider error: {0}")]
    KeyProvider(#[from] super::key_provider::KeyProviderError),
    /// Any other internal failure.
    #[error("verification error: {0}")]
    Internal(String),
}

/// Abstraction over token verification.
///
/// Implementations wrap a concrete token library (e.g. PASETO via `pasetors`).
/// Swapping the library or the authentication mechanism only requires a new
/// implementation of this trait; the middleware and handlers are unchanged.
#[async_trait::async_trait]
pub trait TokenVerifier: Send + Sync {
    /// Verify a raw token string and return the authenticated principal.
    async fn verify(&self, token: &str) -> Result<AuthPrincipal, VerifyError>;

    /// Whether the `Authorization` header may be omitted entirely. Only
    /// development verifiers (e.g. [`StaticUserVerifier`]) return `true`.
    fn allows_anonymous(&self) -> bool {
        false
    }
}

/// Development-only verifier that accepts any token and always returns a
/// fixed principal. Enabled via `AUTH_SKIP_VERIFY=1`; never use outside
/// local testing.
#[derive(Debug, Clone)]
pub struct StaticUserVerifier {
    user_id: uuid::Uuid,
}

impl StaticUserVerifier {
    /// Falls back to a fixed well-known user id when none is configured, so
    /// local runs only need a single env var (`AUTH_SKIP_VERIFY=1`).
    pub fn new(user_id: Option<uuid::Uuid>) -> Self {
        Self {
            user_id: user_id.unwrap_or_else(default_user_id),
        }
    }
}

/// Deterministic fallback principal for local development.
pub fn default_user_id() -> uuid::Uuid {
    "00000000-0000-0000-0000-000000000001"
        .parse()
        .expect("hard-coded default user id is valid")
}

#[async_trait::async_trait]
impl TokenVerifier for StaticUserVerifier {
    async fn verify(&self, _token: &str) -> Result<AuthPrincipal, VerifyError> {
        Ok(AuthPrincipal {
            user_id: self.user_id,
        })
    }

    fn allows_anonymous(&self) -> bool {
        true
    }
}
