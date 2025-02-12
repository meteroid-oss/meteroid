use common_config::telemetry::TelemetryConfig;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::Resource;
use std::time::Duration;

pub fn init_telemetry_metrics(config: &TelemetryConfig) {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(config.otel_endpoint.clone())
        .with_protocol(Protocol::Grpc)
        .with_timeout(Duration::from_secs(3))
        .build()
        .unwrap();

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(5))
        .build();

    let resource = Resource::builder()
        .with_attributes(vec![KeyValue::new("host", "localhost")])
        .build();

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    global::set_meter_provider(provider);
}
