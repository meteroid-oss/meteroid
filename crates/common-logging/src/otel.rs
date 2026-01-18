use opentelemetry::global;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use std::time::Duration;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer};

use crate::logging::formatting_layer;
use common_config::telemetry::TelemetryConfig;
use opentelemetry_resource_detectors::{HostResourceDetector, K8sResourceDetector};
use opentelemetry_sdk::{Resource, resource::ResourceDetector};

fn get_resource() -> Resource {
    let detectors: Vec<Box<dyn ResourceDetector>> = vec![
        Box::new(HostResourceDetector::default()),
        Box::new(K8sResourceDetector),
    ];

    Resource::builder().with_detectors(&detectors).build()
}

pub fn init_meter_provider(cfg: &TelemetryConfig) -> opentelemetry_sdk::metrics::SdkMeterProvider {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(cfg.otel_endpoint.as_str())
        .with_protocol(Protocol::Grpc)
        .with_timeout(Duration::from_secs(3))
        .build()
        .expect("failed to build OTLP metric exporter");

    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(5))
        .build();

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(get_resource())
        .build();

    global::set_meter_provider(provider.clone());

    provider
}

pub fn init_otel_tracing_and_logging() {
    let (trace_layer, _guard) =
        init_tracing_opentelemetry::tracing_subscriber_ext::build_tracer_layer_with_resource(
            get_resource(),
        )
        .expect("failed to build tracing layer");

    let logger_provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_resource(get_resource())
        .with_batch_exporter(
            opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .build()
                .expect("Failed to initialize logger provider"),
        )
        .build();

    let otel_layer = OpenTelemetryTracingBridge::new(&logger_provider);
    let filter_otel = EnvFilter::new("info");
    let otel_layer = otel_layer.with_filter(filter_otel);

    tracing_subscriber::registry()
        .with(trace_layer)
        .with(otel_layer)
        .with(
            init_tracing_opentelemetry::tracing_subscriber_ext::build_level_filter_layer("")
                .unwrap_or_default(),
        )
        .with(formatting_layer())
        .init();
}
