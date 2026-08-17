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
}
