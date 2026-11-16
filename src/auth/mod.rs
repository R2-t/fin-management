pub mod key_provider;
pub mod paseto;
pub mod password;
pub mod service_token;
pub mod session;
pub mod verifier;

use axum::{
    body::Body,
    extract::{Extension, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::config::auth::AuthConfig;
use verifier::{TokenVerifier, VerifyError};

/// Authenticated principal, whatever path established it.
#[derive(Clone, Debug)]
pub struct AuthPrincipal {
    pub user_id: uuid::Uuid,
}

/// How the principal was established. Handlers that need a browser session
/// (logout, for one) must not accept a machine token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthPath {
    Session,
    Bearer,
    Bypass,
}

/// Inserts the principal in both shapes.
///
/// `AuthPrincipal` is what every existing handler extracts (and inserting it is
/// what keeps "one principal regardless of path" true); `AuthContext` adds how
/// it was established, for the endpoints that must insist on a session.
fn attach(req: &mut Request<Body>, principal: AuthPrincipal, path: AuthPath) {
    req.extensions_mut().insert(AuthContext {
        principal: principal.clone(),
        path,
    });
    req.extensions_mut().insert(principal);
}

/// Inserts a principal and how it was established.
#[derive(Clone, Debug)]
pub struct AuthContext {
    pub principal: AuthPrincipal,
    pub path: AuthPath,
}

impl AuthContext {
    pub fn user_id(&self) -> uuid::Uuid {
        self.principal.user_id
    }
}

/// Records an authentication failure (never logs the token itself).
fn auth_failure(reason: &'static str) {
    tracing::info!(monotonic_counter.auth_failures_total = 1u64, reason = reason);
    tracing::warn!(reason, "authentication failed");
}

fn unauthorized(reason: &'static str, message: &str) -> Response {
    auth_failure(reason);
    (StatusCode::UNAUTHORIZED, message.to_string()).into_response()
}

/// Reads the session cookie, `__Host-` form first.
///
/// Both names are always read: a cookie set under one scheme must stay visible
/// under the other, or a change of scheme would make an existing cookie look
/// absent and quietly fall through to the bearer path.
fn session_cookie(req: &Request<Body>) -> Option<String> {
    let cookie_header = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())?;

    AuthConfig::COOKIE_NAMES.iter().find_map(|name| {
        cookie_header.split(';').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key.trim() == *name)
                .then(|| value.trim().to_string())
                .filter(|v| !v.is_empty())
        })
    })
}

fn bearer_token(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Auth middleware: two paths, one principal.
///
/// The order is the security property:
///
/// 1. A session cookie is resolved if present, and a bad cookie is **refused**
///    rather than skipped. Falling through would let a bearer token rescue a
///    revoked session, and would let a stale cookie become an anonymous request.
/// 2. The bearer token is consulted only when no cookie was sent.
/// 3. With neither, the anonymous verifier (dev bypass) decides, so the bypass
///    that every local run uses keeps working.
pub async fn auth_middleware(
    Extension(verifier): Extension<Arc<dyn TokenVerifier>>,
    Extension(pool): Extension<crate::PgPool>,
    Extension(config): Extension<AuthConfig>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    if let Some(token) = session_cookie(&req) {
        return match session::resolve(&pool, &token).await {
            Ok(session::SessionLookup::Active(user_id)) => {
                attach(&mut req, AuthPrincipal { user_id }, AuthPath::Session);
                next.run(req).await
            }
            Ok(session::SessionLookup::Expired) => {
                unauthorized("session_expired", "Your session has expired")
            }
            Ok(session::SessionLookup::Absent) => {
                unauthorized("session_invalid", "Your session is no longer valid")
            }
            Err(e) => {
                tracing::error!(error = %e, "session store failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Session lookup failed".to_string(),
                )
                    .into_response()
            }
        };
    }

    if let Some(token) = bearer_token(&req) {
        return match verify_bearer(verifier.as_ref(), &token).await {
            Ok(principal) => {
                attach(&mut req, principal, AuthPath::Bearer);
                next.run(req).await
            }
            Err(response) => *response,
        };
    }

    // No cookie and no header: only a verifier that allows anonymous access
    // (the development bypass) may proceed.
    if verifier.allows_anonymous() {
        return match verifier.verify("").await {
            Ok(principal) => {
                attach(&mut req, principal, AuthPath::Bypass);
                next.run(req).await
            }
            Err(_) => unauthorized("bypass_rejected", "Authentication is misconfigured"),
        };
    }

    let _ = &config;
    unauthorized("no_session", "Missing Authorization header")
}

async fn verify_bearer(
    verifier: &dyn TokenVerifier,
    token: &str,
) -> Result<AuthPrincipal, Box<Response>> {
    match verifier.verify(token).await {
        Ok(p) => Ok(p),
        Err(e) => {
            let reason = match &e {
                VerifyError::InvalidToken => "invalid_token",
                VerifyError::KeyNotFound => "unknown_key",
                VerifyError::KeyProvider(_) => "key_provider",
                VerifyError::Internal(_) => "internal",
            };
            if matches!(e, VerifyError::KeyProvider(_) | VerifyError::Internal(_)) {
                tracing::error!(error = %e, reason, "token verification infrastructure failure");
            }
            Err(Box::new(match e {
                VerifyError::InvalidToken => unauthorized(reason, "Invalid token"),
                VerifyError::KeyNotFound => unauthorized(reason, "Unknown token key"),
                VerifyError::KeyProvider(_) => {
                    auth_failure(reason);
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "Unable to resolve token key".to_string(),
                    )
                        .into_response()
                }
                VerifyError::Internal(_) => {
                    auth_failure(reason);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Token verification failed".to_string(),
                    )
                        .into_response()
                }
            }))
        }
    }
}
