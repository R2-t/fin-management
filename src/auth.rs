use axum::{
    body::Body,
    extract::Extension,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

/// Authenticated principal extracted from a validated PASETO token.
#[derive(Clone, Debug)]
pub struct AuthPrincipal {
    pub user_id: uuid::Uuid,
}

/// Auth middleware.
///
/// This service does **not** issue tokens. It only verifies that an incoming
/// request presents a structurally valid PASETO token in the `Authorization`
/// header, and exposes the authenticated `user_id` to downstream handlers via
/// `Extension<AuthPrincipal>`.
///
/// The `user_id` is currently read from the `X-Authenticated-User` header set
/// by the upstream gateway/auth service. This avoids fabricating a principal
/// from an unverified (still-encrypted) local token payload.
///
/// TODO(auth): Replace this with full `pasetors::local::decrypt` / `public::verify`
/// against the gateway's shared key once it is provisioned, deriving `user_id`
/// from the verified token claims instead of a trusted header.
pub async fn auth_middleware(
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, (StatusCode, String)> {
    // The gateway must present a PASETO token.
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing Authorization header".into()))?;

    // Structural validation: the token must parse as a valid PASETO local token.
    pasetors::token::UntrustedToken::<pasetors::Local, pasetors::version4::V4>::try_from(
        auth_header,
    )
    .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid PASETO token".into()))?;

    // Trusted user id provided by the upstream auth/gateway layer.
    let user_id = req
        .headers()
        .get("X-Authenticated-User")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<uuid::Uuid>().ok())
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing X-Authenticated-User header".into()))?;

    req.extensions_mut()
        .insert(Extension(AuthPrincipal { user_id }));

    Ok(next.run(req).await)
}
