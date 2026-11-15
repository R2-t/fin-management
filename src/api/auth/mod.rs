use axum::{
    extract::{Extension, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{password, session, AuthContext, AuthPath};
use crate::config::auth::AuthConfig;

/// Login failures are their own type: `VerifyError` is matched exhaustively by
/// the auth middleware against a status table a credentials failure must never
/// reach.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("Email or password is incorrect")]
    InvalidCredentials,
    #[error("Too many attempts; try again later")]
    RateLimited,
    #[error("Password must be at least {0} characters")]
    WeakPassword(usize),
    #[error("That email is already registered")]
    EmailTaken,
    #[error("Request origin is not allowed")]
    Csrf,
    #[error("{0}")]
    BadRequest(String),
    #[error("Internal error")]
    Internal,
}

impl IntoResponse for LoginError {
    fn into_response(self) -> Response {
        let reason = match &self {
            LoginError::InvalidCredentials => "invalid_credentials",
            LoginError::RateLimited => "rate_limited",
            LoginError::WeakPassword(_) => "weak_password",
            LoginError::EmailTaken => "email_taken",
            LoginError::Csrf => "csrf",
            LoginError::BadRequest(_) => "bad_request",
            LoginError::Internal => "internal",
        };
        tracing::info!(monotonic_counter.auth_failures_total = 1u64, reason);

        let status = match &self {
            LoginError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            LoginError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            LoginError::WeakPassword(_) => StatusCode::BAD_REQUEST,
            LoginError::EmailTaken => StatusCode::CONFLICT,
            LoginError::Csrf => StatusCode::FORBIDDEN,
            LoginError::BadRequest(_) => StatusCode::BAD_REQUEST,
            LoginError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };

        // The message a client sees for a rejected credential is deliberately
        // the same one the error carries: it never says whether the email
        // exists, and never echoes the password.
        let body = serde_json::json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}

#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct UserResponse {
    pub user_id: Uuid,
    pub email: Option<String>,
}

/// Normalises an email for lookup and storage. Citext makes the comparison
/// case-insensitive in the database, but trimming here keeps a stray space from
/// creating a second account.
fn normalise_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The address the per-IP limit counts against.
///
/// `X-Forwarded-For` is read only when `TRUST_PROXY=1`: trusting it by default
/// would let anyone bypass the limit with a fresh header value. Without it the
/// socket address is used, which axum only supplies when the server was built
/// with connect info.
///
/// When neither is available the address is `None` and the per-IP limit does not
/// count. That is stated rather than papered over: an address is never invented,
/// so in that configuration only the per-email limit applies.
fn client_ip(
    headers: &HeaderMap,
    config: &AuthConfig,
    connect_info: Option<&std::net::SocketAddr>,
) -> Option<String> {
    if config.trust_proxy {
        if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = forwarded.split(',').next() {
                let first = first.trim();
                if !first.is_empty() {
                    return Some(first.to_string());
                }
            }
        }
    }
    connect_info.map(|addr| addr.ip().to_string())
}

/// CSRF: a cookie-setting endpoint is a target too.
///
/// `SameSite=Strict` stops a cross-site POST from reading the response, but not
/// from setting a cookie: a login-CSRF attack that signs the victim into the
/// attacker's account puts the victim's later writes in the attacker's ledger.
/// A browser always sends `Origin` on a cross-site POST, so a mismatch is a
/// refusal; a missing header is allowed because non-browser clients omit it.
fn check_origin(headers: &HeaderMap, config: &AuthConfig) -> Result<(), LoginError> {
    let Some(expected) = config.app_origin.as_deref() else {
        return Ok(());
    };
    let Some(origin) = headers.get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok())
    else {
        return Ok(());
    };
    if origin == expected {
        Ok(())
    } else {
        Err(LoginError::Csrf)
    }
}

fn cookie_header(config: &AuthConfig, token: &str, max_age_seconds: i64) -> String {
    let mut cookie = format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age_seconds}",
        config.cookie_name()
    );
    if config.secure_cookies {
        cookie.push_str("; Secure");
    }
    cookie
}

fn clear_cookie(config: &AuthConfig) -> String {
    let mut cookie = format!(
        "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        config.cookie_name()
    );
    if config.secure_cookies {
        cookie.push_str("; Secure");
    }
    cookie
}

async fn count_failures(
    pool: &crate::PgPool,
    email: &str,
    ip: Option<&str>,
    config: &AuthConfig,
) -> Result<(i64, i64), LoginError> {
    let conn = pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    let row = conn
        .query_one(
            "SELECT
               (SELECT COUNT(*) FROM login_attempt
                 WHERE email = $1 AND NOT succeeded
                   AND attempted_at > NOW() - make_interval(mins => $2)) AS by_email,
               (SELECT COUNT(*) FROM login_attempt
                 WHERE client_ip = $3::text::inet AND NOT succeeded
                   AND attempted_at > NOW() - make_interval(mins => $2)) AS by_ip",
            &[&email, &(config.login_window_minutes as i32), &ip],
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    Ok((row.get("by_email"), row.get("by_ip")))
}

async fn record_attempt(
    pool: &crate::PgPool,
    email: &str,
    ip: Option<&str>,
    succeeded: bool,
) -> Result<(), LoginError> {
    let conn = pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    conn.execute(
        "INSERT INTO login_attempt (email, client_ip, succeeded) VALUES ($1, $2::text::inet, $3)",
        &[&email, &ip, &succeeded],
    )
    .await
    .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    if succeeded {
        // A success clears the email's failures, so a user who mistyped twice
        // and then got it right is not left inside a counting window.
        conn.execute(
            "DELETE FROM login_attempt WHERE email = $1 AND NOT succeeded",
            &[&email],
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    }
    Ok(())
}

pub async fn register(
    State(state): State<std::sync::Arc<crate::AppState>>,
    Extension(pool): Extension<crate::PgPool>,
    Extension(config): Extension<AuthConfig>,
    headers: HeaderMap,
    Json(body): Json<Credentials>,
) -> Result<Response, LoginError> {
    let _ = state;
    check_origin(&headers, &config)?;
    let email = normalise_email(&body.email);
    if !email.contains('@') {
        return Err(LoginError::BadRequest("That is not an email address".into()));
    }
    if body.password.chars().count() < config.password_min_length {
        return Err(LoginError::WeakPassword(config.password_min_length));
    }

    let hash = password::hash_password(&body.password).map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    let user_id = Uuid::new_v4();
    let conn = pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    let inserted = conn
        .execute(
            "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)
             ON CONFLICT (email) DO NOTHING",
            &[&user_id, &email, &hash],
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    if inserted == 0 {
        return Err(LoginError::EmailTaken);
    }

    let issued = session::issue(&pool, user_id, &config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    tracing::info!(monotonic_counter.auth_registrations_total = 1u64);

    Ok((
        StatusCode::CREATED,
        [(
            axum::http::header::SET_COOKIE,
            cookie_header(&config, &issued.token, config.session_ttl_days * 86_400),
        )],
        Json(UserResponse {
            user_id,
            email: Some(email),
        }),
    )
        .into_response())
}

pub async fn login(
    State(state): State<std::sync::Arc<crate::AppState>>,
    Extension(pool): Extension<crate::PgPool>,
    Extension(config): Extension<AuthConfig>,
    headers: HeaderMap,
    connect_info: Option<Extension<axum::extract::ConnectInfo<std::net::SocketAddr>>>,
    Json(body): Json<Credentials>,
) -> Result<Response, LoginError> {
    let _ = state;
    check_origin(&headers, &config)?;
    let email = normalise_email(&body.email);
    let ip = client_ip(&headers, &config, connect_info.as_ref().map(|c| &c.0 .0));

    let (by_email, by_ip) = count_failures(&pool, &email, ip.as_deref(), &config).await?;
    if by_email >= config.login_max_failures_per_email
        || by_ip >= config.login_max_failures_per_ip
    {
        record_attempt(&pool, &email, ip.as_deref(), false).await?;
        return Err(LoginError::RateLimited);
    }

    let conn = pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    let row = conn
        .query_opt(
            "SELECT id, password_hash FROM users WHERE email = $1",
            &[&email],
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;

    let (user_id, ok) = match row {
        Some(row) => {
            let hash: String = row.get("password_hash");
            let ok = password::verify_password(&body.password, &hash);
            (row.get::<_, Uuid>("id"), ok)
        }
        None => {
            // Burn the same work a real verification would, so a missing email
            // is not distinguishable from a wrong password by timing.
            password::verify_against_dummy(&body.password);
            (Uuid::nil(), false)
        }
    };

    record_attempt(&pool, &email, ip.as_deref(), ok).await?;
    if !ok {
        return Err(LoginError::InvalidCredentials);
    }

    // A login never reuses a cookie value: the previous session, if any, is
    // revoked and a new one issued. That is the fixation fix.
    let previous = session_cookie_value(&headers);
    let issued = session::replace(&pool, user_id, previous.as_deref(), &config)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    tracing::info!(monotonic_counter.auth_logins_total = 1u64);

    Ok((
        StatusCode::OK,
        [(
            axum::http::header::SET_COOKIE,
            cookie_header(&config, &issued.token, config.session_ttl_days * 86_400),
        )],
        Json(UserResponse {
            user_id,
            email: Some(email),
        }),
    )
        .into_response())
}

pub async fn logout(
    Extension(pool): Extension<crate::PgPool>,
    Extension(config): Extension<AuthConfig>,
    Extension(ctx): Extension<AuthContext>,
    headers: HeaderMap,
) -> Result<Response, LoginError> {
    if ctx.path != AuthPath::Session {
        return Err(LoginError::BadRequest(
            "Logout needs a session, not a bearer token".into(),
        ));
    }
    if let Some(token) = session_cookie_value(&headers) {
        session::revoke(&pool, &token)
            .await
            .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    }
    tracing::info!(monotonic_counter.auth_logouts_total = 1u64);
    Ok((
        StatusCode::NO_CONTENT,
        [(axum::http::header::SET_COOKIE, clear_cookie(&config))],
    )
        .into_response())
}

pub async fn me(
    Extension(pool): Extension<crate::PgPool>,
    Extension(ctx): Extension<AuthContext>,
) -> Result<Response, LoginError> {
    let conn = pool.get().await.map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    let row = conn
        .query_opt("SELECT email FROM users WHERE id = $1", &[&ctx.user_id()])
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "auth: database error");
            LoginError::Internal
        })?;
    // A bearer principal may have no users row -- a machine token for a
    // fabricated test user is legitimate -- so the email is absent rather than
    // an error.
    let email: Option<String> = row.map(|r| r.get("email"));
    Ok(Json(UserResponse {
        user_id: ctx.user_id(),
        email,
    })
    .into_response())
}

fn session_cookie_value(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers
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
