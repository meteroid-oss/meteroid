use crate::init_metrics::init_telemetry_metrics;
use common_config::telemetry::TelemetryConfig;
use tracing::{log, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

pub fn init_telemetry(config: &TelemetryConfig, service_name: &str) {
    if config.tracing_enabled {
        init_telemetry_tracing(config, service_name);
        log::info!("Tracing is enabled");
    } else {
        init_regular_logging();
        log::info!("Tracing is disabled");
    }

    if config.metrics_enabled {
        init_telemetry_metrics(config);
        log::info!("Metrics is enabled");
    } else {
        log::warn!("Metrics is disabled");
    }
}

pub fn init_regular_logging() {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(formatting_layer())
        .with(filter)
        .init();
}

fn init_telemetry_tracing(config: &TelemetryConfig, service_name: &str) {
    // setting env variables needed by [init_tracing_opentelemetry]
    //   see also
    //     https://lib.rs/crates/init-tracing-opentelemetry
    //     https://opentelemetry.io/docs/concepts/sdk-configuration/otlp-exporter-configuration/
    //
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", config.otel_endpoint.clone());
    std::env::set_var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc");

    std::env::set_var("OTEL_SERVICE_NAME", service_name);

    let (layer, _guard) =
        init_tracing_opentelemetry::tracing_subscriber_ext::build_otel_layer().unwrap();

    tracing_subscriber::registry()
        .with(layer)
        .with(init_tracing_opentelemetry::tracing_subscriber_ext::build_loglevel_filter_layer())
        .with(formatting_layer())
        .try_init()
        .unwrap()
}

fn formatting_layer<S>() -> Box<dyn tracing_subscriber::layer::Layer<S> + Send + Sync + 'static>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    Box::new(
        tracing_subscriber::fmt::Layer::default()
            .with_ansi(true)
            .with_target(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_thread_names(true)
            .compact(),
    )
}
