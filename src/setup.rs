use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::{
    trace::{self, RandomIdGenerator},
    Resource,
};
use std::time::Duration;
use tonic::metadata::MetadataMap;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn init_tracing(endpoint: String, timeout: Duration, host: String, service_name: String) {
    let mut map = MetadataMap::with_capacity(1);

    map.insert("x-host", host.parse().unwrap());

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(timeout)
        .with_metadata(map)
        .build()
        .unwrap();

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_config(
            trace::Config::default()
                .with_sampler(trace::Sampler::AlwaysOn)
                .with_id_generator(RandomIdGenerator::default())
                .with_max_events_per_span(32)
                .with_max_attributes_per_span(16)
                .with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    service_name,
                )])),
        )
        .build();

    let tracer = provider.tracer("fin-management");

    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry().with(telemetry).init();
}
