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
        // TCP keepalives: a half-open socket (NAT/proxy dropping the flow) would
        // otherwise leave the LISTEN connection silently dead but "open".
        config
            .keepalives(true)
            .keepalives_idle(Duration::from_secs(30))
            .keepalives_interval(Duration::from_secs(10))
            .keepalives_retries(3);
        config
    }

    pub async fn get_pool(&self) -> Pool<PostgresConnectionManager<NoTls>> {
        let pg_cm = PostgresConnectionManager::new(self.get_config(), NoTls);

        // Fail fast when the DB is unreachable instead of hanging for bb8's
        // 30s default; surfaces as `repository_errors_total{kind="pool_timeout"}`.
        Pool::builder()
            .connection_timeout(std::time::Duration::from_secs(5))
            .build(pg_cm)
            .await
            .unwrap()
    }
}
