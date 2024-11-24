use opentelemetry_sdk::logs::Logger;
use otlpconfig::{LogConfig, TracerConfig};
use tracing_subscriber::{
    filter::LevelFilter, layer::Filter, layer::SubscriberExt, util::SubscriberInitExt,
};

pub mod otlpconfig;

pub struct Config {
    pub otlpconfig: otlpconfig::Config,
}

impl Config {
    pub fn init(&self) -> Logger {
        let tracer = self.otlpconfig.create_tracer();
        let logger = self.otlpconfig.initialize_logger();
        let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

        tracing_subscriber::registry()
            .with(tracing_subscriber::filter::dynamic_filter_fn(
                move |metadata, cx| {
                    if metadata.is_span() && LevelFilter::DEBUG.enabled(metadata, &cx) {
                        return true;
                    }
                    if metadata.is_event() && LevelFilter::DEBUG.enabled(metadata, &cx) {
                        return true;
                    }
                    false
                },
            ))
            .with(telemetry)
            .init();
        logger
    }
}
