/// Auth settings, read once at startup.
///
/// Every number here is named in `docs/plan/07-auth.md` with its default and
/// the reason for it, so an operator can find them in one place instead of in
/// prose.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub session_ttl_days: i64,
    pub login_max_failures_per_email: i64,
    pub login_max_failures_per_ip: i64,
    pub login_window_minutes: i64,
    pub password_min_length: usize,
    pub paseto_audience: String,
    pub paseto_issuer: String,
    /// The origin CSRF checks compare against. `None` disables the check, which
    /// is only acceptable when cookies are off entirely.
    pub app_origin: Option<String>,
    pub secure_cookies: bool,
    pub trust_proxy: bool,
    pub service_token_ttl_seconds: i64,
    pub job_token_ttl_seconds: i64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_ttl_days: 30,
            login_max_failures_per_email: 5,
            login_max_failures_per_ip: 20,
            login_window_minutes: 15,
            password_min_length: 12,
            paseto_audience: "financial-management".into(),
            paseto_issuer: "financial-management".into(),
            app_origin: None,
            secure_cookies: false,
            trust_proxy: false,
            service_token_ttl_seconds: 60,
            job_token_ttl_seconds: 300,
        }
    }
}

impl AuthConfig {
    pub fn cookie_name(&self) -> &'static str {
        if self.secure_cookies {
            // The `__Host-` prefix requires Secure, Path=/ and no Domain.
            "__Host-fm_session"
        } else {
            "fm_session"
        }
    }

    /// Both names are always read. A cookie set under one scheme must stay
    /// visible under the other, or a change of scheme would make an existing
    /// cookie look absent and fall through to the bearer path.
    pub const COOKIE_NAMES: [&'static str; 2] = ["__Host-fm_session", "fm_session"];
}
