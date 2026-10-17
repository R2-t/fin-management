use std::time::Duration;

use bb8_postgres::{bb8::Pool, PostgresConnectionManager};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use otlpconfig::{LogConfig, MetricConfig, TracerConfig};
use tokio_postgres::NoTls;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::{
    layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
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
        // The OpenTelemetry crates' own records are excluded from this bridge:
        // a failing exporter would otherwise log into the exporter it is
        // failing to reach. They still reach the console layer.
        let log_layer = OpenTelemetryTracingBridge::new(&self.otlpconfig.create_log_provider())
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                !metadata.target().starts_with("opentelemetry")
            }));
        // Layer 3: metrics from events using counter./histogram. prefixes.
        // The layer does not retain the provider; dropping the last handle shuts
        // the periodic exporter down (no metrics ever leave the process). The
        // global registration keeps a clone alive for the process lifetime.
        opentelemetry::global::set_meter_provider(meter_provider.clone());
        let metrics_layer = MetricsLayer::new(meter_provider);

        // Global filter. Override with RUST_LOG, e.g.
        // `RUST_LOG=info,financial_management=debug,tower_http=debug`.
        // Noisy transport crates are kept at WARN so the OTLP exporters cannot
        // feed their own logs back into the log pipeline.
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,h2=warn,hyper=warn,hyper_util=warn,tonic=warn,tower=warn,opentelemetry=warn,opentelemetry_sdk=warn,opentelemetry_otlp=warn")
        });

        // Console layer so `docker logs` works even when the collector is down.
        // LOG_FORMAT=json for structured output; anything else is human readable.
        let json = std::env::var("LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json"));
        let (fmt_json, fmt_text) = if json {
            (Some(tracing_subscriber::fmt::layer().json()), None)
        } else {
            (None, Some(tracing_subscriber::fmt::layer()))
        };

        // W3C trace-context propagator. Used for inbound `traceparent` extraction;
        // outbound injection is not implemented yet (no outbound HTTP client
        // exists until the Python sidecar lands).
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );

        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_json)
            .with(fmt_text)
            .with(trace_layer)
            .with(log_layer)
            .with(metrics_layer)
            .init();

        // Route panics (including in background tasks spawned below and in
        // main) through tracing; installed before any task starts.
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(panic = %info, "panic");
            default_hook(info);
        }));

        spawn_pool_metrics(db_pool.clone());

        db_pool
    }
}

/// Samples bb8 pool state into gauges every 15s so pool saturation is visible
/// (`db_pool_connections{state}`; alert when `idle == 0` for a sustained time).
fn spawn_pool_metrics(pool: Pool<PostgresConnectionManager<NoTls>>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            let st = pool.state();
            let total = st.connections as u64;
            let idle = st.idle_connections as u64;
            tracing::info!(gauge.db_pool_connections = idle, pool_state = "idle");
            tracing::info!(
                gauge.db_pool_connections = total.saturating_sub(idle),
                pool_state = "in_use"
            );
        }
    });
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

/// How long processed outbox rows are retained before the periodic purge.
pub fn get_purge_keep_days() -> u64 {
    std::env::var("PURGE_KEEP_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7)
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
