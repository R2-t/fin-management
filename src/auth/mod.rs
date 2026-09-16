pub mod key_provider;
pub mod paseto;
pub mod verifier;

use axum::{
    body::Body,
    extract::Extension,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

use verifier::{TokenVerifier, VerifyError};

/// Authenticated principal extracted from a verified token.
#[derive(Clone, Debug)]
pub struct AuthPrincipal {
    pub user_id: uuid::Uuid,
}

/// Auth middleware.
///
/// This service does **not** issue tokens. It only verifies that an incoming
/// request presents a valid token in the `Authorization` header, using the
/// configured [`TokenVerifier`], and exposes the authenticated `user_id` to
/// downstream handlers via `Extension<AuthPrincipal>`.
///
/// The verifier is injected via `Extension<Arc<dyn TokenVerifier>>` so the
/// concrete token library (PASETO today, something else tomorrow) is swappable
/// without touching the middleware or handlers.
pub async fn auth_middleware(
    Extension(verifier): Extension<Arc<dyn TokenVerifier>>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    // Verifiers that allow anonymous access (local dev bypass) may omit the
    // header entirely; the token is then irrelevant.
    let token = match token {
        Some(t) => t,
        None if verifier.allows_anonymous() => "",
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                "Missing Authorization header".into(),
            ))
        }
    };

    let principal = match verifier.verify(token).await {
        Ok(p) => p,
        Err(e) => {
            return Err(match e {
                VerifyError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token".into()),
                VerifyError::KeyNotFound => (StatusCode::UNAUTHORIZED, "Unknown token key".into()),
                VerifyError::KeyProvider(_) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Unable to resolve token key".into(),
                ),
                VerifyError::Internal(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Token verification failed".into(),
                ),
            })
        }
    };

    // Inserted bare (not wrapped in `Extension`) so handlers can extract
    // `Extension<AuthPrincipal>` — `Extensions` keys by the concrete type.
    req.extensions_mut().insert(AuthPrincipal {
        user_id: principal.user_id,
    });

    Ok(next.run(req).await)
}
