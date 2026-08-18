use std::time::Duration;

use bb8_postgres::{bb8::Pool, PostgresConnectionManager};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use otlpconfig::{LogConfig, MetricConfig, TracerConfig};
use tokio_postgres::NoTls;
use tracing_opentelemetry::MetricsLayer;
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

    pub fn init(&self) -> Pool<PostgresConnectionManager<NoTls>> {
        let tracer = self.otlpconfig.create_tracer();
        let meter_provider = self.otlpconfig.create_meter_provider();
        let db_pool = futures::executor::block_on(self.postgresconfig.get_pool());

        // Layer 1: traces (tracing-opentelemetry).
        let trace_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        // Layer 2: logs with trace-id/span-id correlation to the active span.
        let log_layer = OpenTelemetryTracingBridge::new(&self.otlpconfig.create_log_provider());
        // Layer 3: metrics from events using counter./histogram. prefixes.
        let metrics_layer = MetricsLayer::new(meter_provider);

        let filter = tracing_subscriber::filter::dynamic_filter_fn(move |metadata, cx| {
            if metadata.is_span() && LevelFilter::INFO.enabled(metadata, cx) {
                return true;
            }
            if metadata.is_event() && LevelFilter::INFO.enabled(metadata, cx) {
                return true;
            }
            false
        });

        tracing_subscriber::registry()
            .with(filter)
            .with(trace_layer)
            .with(log_layer)
            .with(metrics_layer)
            .init();

        db_pool
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
