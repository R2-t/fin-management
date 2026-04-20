use std::str::FromStr;

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

    fn get_config(&self) -> tokio_postgres::Config {
        tokio_postgres::Config::from_str(&self.get_connection_string()).unwrap()
    }

    pub async fn get_pool(&self) -> Pool<PostgresConnectionManager<NoTls>> {
        let pg_cm = PostgresConnectionManager::new(self.get_config(), NoTls);

        Pool::builder().build(pg_cm).await.unwrap()
    }
}
