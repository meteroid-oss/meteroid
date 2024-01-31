use std::time::Duration;

use common_config::telemetry::TelemetryConfig;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{runtime, Resource};

pub fn init_telemetry_metrics(config: &TelemetryConfig) {
    let provider = opentelemetry_otlp::new_pipeline()
        .metrics(runtime::Tokio)
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(config.otel_endpoint.clone())
                .with_protocol(Protocol::Grpc),
        )
        .with_period(Duration::from_secs(10))
        .with_timeout(Duration::from_secs(10))
        .with_resource(Resource::new(vec![KeyValue::new(
            "host",
            "localhost", // todo update
        )]))
        .build()
        .expect("Failed to setup metrics pipeline");

    global::set_meter_provider(provider);
}
