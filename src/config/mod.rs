use std::time::Duration;

use bb8_postgres::{bb8::Pool, PostgresConnectionManager};
use otlpconfig::{LogConfig, LoggerWrapper, TracerConfig};
use tokio_postgres::NoTls;
use tracing_subscriber::{
    filter::LevelFilter, layer::Filter, layer::SubscriberExt, util::SubscriberInitExt,
};

pub mod otlpconfig;
pub mod postgresconfig;

// Environment variables
const ENV_SERVICE_NAME: &str = "SERVICE_NAME";
const ENV_OTLP_ENDPOINT: &str = "OTLP_ENDPOINT";

// contants config
const OTLP_TIMEOUT_SECS: u64 = 10;

pub struct Config {
    pub otlpconfig: otlpconfig::Config,
    pub postgresconfig: postgresconfig::Config,
}

impl Config {
    pub fn new() -> Self {
        let otlpconfig = otlpconfig::Config::new(
            get_otlp_endpoint(),
            Duration::from_secs(OTLP_TIMEOUT_SECS),
            get_service_name(),
        );

        let postgresconfig = postgresconfig::Config::new(
            get_database_host(),
            get_database_username(),
            get_database_password(),
            get_database_name(),
        );

        Self {
            otlpconfig,
            postgresconfig,
        }
    }

    pub fn init(&self) -> (LoggerWrapper, Pool<PostgresConnectionManager<NoTls>>) {
        let tracer = self.otlpconfig.create_tracer();
        let logger = self.otlpconfig.initialize_logger();
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        let db_pool = futures::executor::block_on(self.postgresconfig.get_pool());

        tracing_subscriber::registry()
            .with(tracing_subscriber::filter::dynamic_filter_fn(
                move |metadata, cx| {
                    if metadata.is_span() && LevelFilter::INFO.enabled(metadata, cx) {
                        return true;
                    }
                    if metadata.is_event() && LevelFilter::INFO.enabled(metadata, cx) {
                        return true;
                    }
                    false
                },
            ))
            .with(telemetry)
            .init();

        (logger, db_pool)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

pub fn get_service_name() -> String {
    std::env::var(ENV_SERVICE_NAME).unwrap_or("fin-management".to_string())
}

pub fn get_otlp_endpoint() -> String {
    std::env::var(ENV_OTLP_ENDPOINT).unwrap_or("http://localhost:4317".to_string())
}

pub fn get_database_username() -> String {
    std::env::var("DB_USERNAME").unwrap_or("postgres".to_string())
}

pub fn get_database_password() -> String {
    std::env::var("DB_PASSWORD").unwrap_or("postgres".to_string())
}

pub fn get_database_host() -> String {
    std::env::var("DB_HOST").unwrap_or("localhost".to_string())
}

pub fn get_database_name() -> String {
    std::env::var("DB_NAME").unwrap_or("postgres".to_string())
}
