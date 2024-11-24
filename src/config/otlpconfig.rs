use opentelemetry::logs::LoggerProvider;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::logs::Logger;
use opentelemetry_sdk::{
    trace::{self, RandomIdGenerator},
    Resource,
};
use std::time::Duration;
use tonic::metadata::MetadataMap;

#[allow(dead_code)]
pub trait TracerConfig {
    type SpanExporter: opentelemetry_sdk::export::trace::SpanExporter;
    fn create_span_exporter(&self) -> Self::SpanExporter;
    fn create_trace_provider(&self) -> opentelemetry_sdk::trace::TracerProvider;
    fn create_tracer(&self) -> trace::Tracer;
}

#[allow(dead_code)]
pub trait LogConfig {
    type LogExporter: opentelemetry_sdk::export::logs::LogExporter;
    fn create_log_exporter(&self) -> Self::LogExporter;
    fn create_log_provider(&self) -> opentelemetry_sdk::logs::LoggerProvider;
    fn initialize_logger(&self) -> Logger;
}

#[allow(dead_code)]
pub struct Config {
    otlp_endpoint: String,
    otlp_timeout: Duration,
    service_name: String,
}

impl Config {
    pub fn new(otlp_endpoint: String, otlp_timeout: Duration, service_name: String) -> Self {
        Self {
            otlp_endpoint,
            otlp_timeout,
            service_name,
        }
    }
}

#[cfg(not(feature = "local"))]
impl TracerConfig for Config {
    type SpanExporter = opentelemetry_otlp::SpanExporter;

    fn create_span_exporter(&self) -> Self::SpanExporter {
        let mut map = MetadataMap::with_capacity(2);

        map.insert("x-host", self.service_name.parse().unwrap());
        map.insert("x-service-name", self.service_name.parse().unwrap());

        opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(self.otlp_endpoint.clone())
            .with_timeout(self.otlp_timeout)
            .with_metadata(map)
            .build()
            .unwrap()
    }

    fn create_trace_provider(&self) -> opentelemetry_sdk::trace::TracerProvider {
        let exporter = self.create_span_exporter();

        opentelemetry_sdk::trace::TracerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_config(
                trace::Config::default()
                    .with_sampler(trace::Sampler::AlwaysOn)
                    .with_id_generator(RandomIdGenerator::default())
                    .with_max_events_per_span(32)
                    .with_max_attributes_per_span(16)
                    .with_resource(Resource::new(vec![KeyValue::new(
                        "service.name",
                        self.service_name.clone(),
                    )])),
            )
            .build()
    }

    fn create_tracer(&self) -> trace::Tracer {
        let provider = self.create_trace_provider();
        provider.tracer(self.service_name.clone())
    }
}

#[cfg(not(feature = "local"))]
impl LogConfig for Config {
    type LogExporter = opentelemetry_otlp::LogExporter;

    fn create_log_exporter(&self) -> Self::LogExporter {
        let mut map = MetadataMap::with_capacity(2);

        map.insert("x-host", self.service_name.parse().unwrap());
        map.insert("x-service-name", self.service_name.parse().unwrap());

        opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_endpoint(self.otlp_endpoint.clone())
            .with_timeout(self.otlp_timeout)
            .with_metadata(map)
            .build()
            .unwrap()
    }

    fn create_log_provider(&self) -> opentelemetry_sdk::logs::LoggerProvider {
        let exporter = self.create_log_exporter();

        opentelemetry_sdk::logs::LoggerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .build()
    }

    fn initialize_logger(&self) -> Logger {
        let logger_provider = self.create_log_provider();

        let logger = logger_provider.logger(self.service_name.clone());

        logger
    }
}

#[cfg(feature = "local")]
impl TracerConfig for Config {
    type SpanExporter = opentelemetry_stdout::SpanExporter;

    fn create_span_exporter(&self) -> Self::SpanExporter {
        opentelemetry_stdout::SpanExporter::default()
    }

    fn create_trace_provider(&self) -> opentelemetry_sdk::trace::TracerProvider {
        let exporter = self.create_span_exporter();

        opentelemetry_sdk::trace::TracerProvider::builder()
            .with_simple_exporter(exporter)
            .build()
    }

    fn create_tracer(&self) -> trace::Tracer {
        let provider = self.create_trace_provider();

        provider.tracer(self.service_name.clone())
    }
}

#[cfg(feature = "local")]
impl LogConfig for Config {
    type LogExporter = opentelemetry_stdout::LogExporter;

    fn create_log_exporter(&self) -> Self::LogExporter {
        opentelemetry_stdout::LogExporter::default()
    }

    fn create_log_provider(&self) -> opentelemetry_sdk::logs::LoggerProvider {
        let exporter = self.create_log_exporter();

        opentelemetry_sdk::logs::LoggerProvider::builder()
            .with_simple_exporter(exporter)
            .build()
    }

    fn initialize_logger(&self) -> Logger {
        let logger_provider = self.create_log_provider();
        logger_provider.logger(self.service_name.clone())
    }
}
