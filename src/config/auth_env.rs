use std::collections::HashMap;

use super::auth::AuthConfig;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn var_or(name: &str, default: &str) -> String {
    var(name).unwrap_or_else(|| default.to_string())
}

fn var_parse<T: std::str::FromStr>(name: &str, default: T) -> T {
    var(name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl AuthConfig {
    /// Reads the auth settings. Every default is named in
    /// `docs/plan/07-auth.md`, so none of these numbers is a surprise.
    ///
    /// `APP_ORIGIN` has no default on purpose: it is the CSRF anchor, and a
    /// guessed origin is a wrong origin.
    pub fn from_env() -> Self {
        Self {
            session_ttl_days: var_parse("SESSION_TTL_DAYS", 30),
            login_max_failures_per_email: var_parse("LOGIN_MAX_FAILURES_PER_EMAIL", 5),
            login_max_failures_per_ip: var_parse("LOGIN_MAX_FAILURES_PER_IP", 20),
            login_window_minutes: var_parse("LOGIN_WINDOW_MINUTES", 15),
            password_min_length: var_parse("PASSWORD_MIN_LENGTH", 12),
            paseto_audience: var_or("PASETO_AUDIENCE", "financial-management"),
            paseto_issuer: var_or("PASETO_ISSUER", "financial-management"),
            app_origin: var("APP_ORIGIN"),
            secure_cookies: var("APP_SECURE_COOKIES").is_some_and(|v| v == "1"),
            trust_proxy: var("TRUST_PROXY").is_some_and(|v| v == "1"),
            service_token_ttl_seconds: var_parse("SERVICE_TOKEN_TTL_SECONDS", 60),
            job_token_ttl_seconds: var_parse("JOB_TOKEN_TTL_SECONDS", 300),
        }
    }

    /// Reads every `PASETO_KEY_<KID>` in the environment, so keys can be rotated
    /// by setting a new active kid without a code change.
    pub fn keys_from_env() -> HashMap<String, Vec<u8>> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let mut keys = HashMap::new();
        for (name, value) in std::env::vars() {
            let Some(kid) = name.strip_prefix("PASETO_KEY_") else {
                continue;
            };
            let kid = kid.to_ascii_lowercase();
            match STANDARD.decode(value.trim()) {
                Ok(bytes) if bytes.len() == 32 => {
                    keys.insert(kid, bytes);
                }
                Ok(bytes) => {
                    eprintln!(
                        "PASETO_KEY_{} is {} bytes; a v4.local key is 32. Ignored.",
                        kid.to_ascii_uppercase(),
                        bytes.len()
                    );
                }
                Err(_) => {
                    eprintln!(
                        "PASETO_KEY_{} is not valid base64. Ignored.",
                        kid.to_ascii_uppercase()
                    );
                }
            }
        }
        keys
    }

    pub fn active_key_id() -> Option<String> {
        var("PASETO_ACTIVE_KID").map(|v| v.to_ascii_lowercase())
    }

    pub fn service_key_id() -> Option<String> {
        var("PASETO_SERVICE_KID").map(|v| v.to_ascii_lowercase())
    }
}
