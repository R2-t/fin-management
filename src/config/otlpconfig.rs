use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::{
    metrics::exporter::PushMetricExporter,
    metrics::SdkMeterProvider,
    trace::{self, RandomIdGenerator},
    Resource,
};
use std::time::Duration;
use tonic::metadata::MetadataMap;

#[allow(dead_code)]
pub trait TracerConfig {
    type SpanExporter: opentelemetry_sdk::trace::SpanExporter;
    fn create_span_exporter(&self) -> Self::SpanExporter;
    fn create_trace_provider(&self) -> opentelemetry_sdk::trace::SdkTracerProvider;
    fn create_tracer(&self) -> trace::Tracer;
}

#[allow(dead_code)]
pub trait MetricConfig {
    type MetricExporter: PushMetricExporter;
    fn create_metric_exporter(&self) -> Self::MetricExporter;
    fn create_meter_provider(&self) -> SdkMeterProvider;
}

#[allow(dead_code)]
pub trait LogConfig {
    type LogExporter: opentelemetry_sdk::logs::LogExporter;
    fn create_log_exporter(&self) -> Self::LogExporter;
    fn create_log_provider(&self) -> opentelemetry_sdk::logs::SdkLoggerProvider;
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

    fn create_trace_provider(&self) -> opentelemetry_sdk::trace::SdkTracerProvider {
        let exporter = self.create_span_exporter();

        opentelemetry_sdk::trace::TracerProviderBuilder::default()
            .with_batch_exporter(exporter)
            .with_sampler(trace::Sampler::ParentBased(Box::new(
                trace::Sampler::TraceIdRatioBased(
                    std::env::var("TRACE_SAMPLE_RATIO")
                        .ok()
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(1.0),
                ),
            )))
            .with_id_generator(RandomIdGenerator::default())
            .with_max_events_per_span(32)
            .with_max_attributes_per_span(16)
            .with_resource(
                Resource::builder()
                    .with_service_name(self.service_name.clone())
                    .build(),
            )
            .build()
    }

    fn create_tracer(&self) -> trace::Tracer {
        let provider = self.create_trace_provider();
        provider.tracer(self.service_name.clone())
    }
}

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

    fn create_log_provider(&self) -> opentelemetry_sdk::logs::SdkLoggerProvider {
        let exporter = self.create_log_exporter();

        opentelemetry_sdk::logs::SdkLoggerProvider::builder()
            .with_batch_exporter(exporter)
            .build()
    }
}

impl MetricConfig for Config {
    type MetricExporter = opentelemetry_otlp::MetricExporter;

    fn create_metric_exporter(&self) -> Self::MetricExporter {
        let mut map = MetadataMap::with_capacity(2);

        map.insert("x-host", self.service_name.parse().unwrap());
        map.insert("x-service-name", self.service_name.parse().unwrap());

        opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(self.otlp_endpoint.clone())
            .with_timeout(self.otlp_timeout)
            .with_metadata(map)
            .build()
            .unwrap()
    }

    fn create_meter_provider(&self) -> SdkMeterProvider {
        let exporter = self.create_metric_exporter();

        opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_periodic_exporter(exporter)
            .with_resource(
                Resource::builder()
                    .with_service_name(self.service_name.clone())
                    .build(),
            )
            .build()
    }
}
