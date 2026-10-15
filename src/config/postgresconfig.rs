use std::str::FromStr;
use std::time::Duration;

use bb8_postgres::{bb8::Pool, PostgresConnectionManager};
use tokio_postgres::NoTls;

pub struct Config {
    pub host: String,
    pub username: String,
    pub password: String,
    pub database: String,
}

impl Config {
    pub fn new(host: String, username: String, password: String, database: String) -> Self {
        Self {
            host,
            username,
            password,
            database,
        }
    }

    fn get_connection_string(&self) -> String {
        format!(
            "postgres://{username}:{password}@{host}:5432/{database}",
            username = self.username,
            password = self.password,
            host = self.host,
            database = self.database
        )
    }

    /// Shared connection settings. `pub` so the LISTEN connection can be opened
    /// outside the bb8 pool (see repository::analytics::listen_forever).
    pub fn get_config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::from_str(&self.get_connection_string()).unwrap();
        apply_transport_settings(&mut config);
        config
    }

    pub async fn get_pool(&self) -> Pool<PostgresConnectionManager<NoTls>> {
        build_pool(self.get_config())
            .await
            .expect("failed to build the database pool")
    }
}

/// TCP keepalives: a half-open socket (NAT or proxy dropping the flow) would
/// otherwise leave a long-lived connection silently dead but "open".
fn apply_transport_settings(config: &mut tokio_postgres::Config) {
    config
        .keepalives(true)
        .keepalives_idle(Duration::from_secs(30))
        .keepalives_interval(Duration::from_secs(10))
        .keepalives_retries(3);
}

/// Builds a pool from an explicit config. Fail fast when the database is
/// unreachable instead of hanging for bb8's 30s default; surfaces as
/// `repository_errors_total{kind="pool_timeout"}`.
async fn build_pool(
    config: tokio_postgres::Config,
) -> Result<Pool<PostgresConnectionManager<NoTls>>, String> {
    let pg_cm = PostgresConnectionManager::new(config, NoTls);
    Pool::builder()
        .connection_timeout(Duration::from_secs(5))
        .build(pg_cm)
        .await
        .map_err(|e| e.to_string())
}

/// Builds a pool from a connection string, without initialising telemetry.
/// Used by integration tests, which cannot call `Config::init`.
pub async fn pool_from_dsn(dsn: &str) -> Result<Pool<PostgresConnectionManager<NoTls>>, String> {
    let mut config = tokio_postgres::Config::from_str(dsn).map_err(|e| e.to_string())?;
    apply_transport_settings(&mut config);
    build_pool(config).await
}
