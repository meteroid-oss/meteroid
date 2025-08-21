use crate::init_metrics::init_telemetry_metrics;
use common_config::telemetry::TelemetryConfig;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::LogExporter;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use tracing::{Subscriber, log};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

pub fn init_telemetry(config: &TelemetryConfig, service_name: &str) {
    if config.tracing_enabled {
        init_telemetry_tracing(config, service_name);
        log::info!("Tracing is enabled");
    } else {
        init_regular_logging();
        log::warn!("Tracing is disabled");
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
    // todo fix unsafe code, move these env vars to.env?
    unsafe {
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", config.otel_endpoint.clone());
        std::env::set_var("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "grpc");
        std::env::set_var("OTEL_SERVICE_NAME", service_name);
    }

    let (otel_layer, _guard) =
        init_tracing_opentelemetry::tracing_subscriber_ext::build_tracer_layer().unwrap();

    let log_exporter = LogExporter::builder().with_tonic().build().unwrap();

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .build();

    let log_otel_layer = OpenTelemetryTracingBridge::new(&logger_provider);

    // For the OpenTelemetry layer, add a tracing filter to filter events from
    // OpenTelemetry and its dependent crates (opentelemetry-otlp uses crates
    // like reqwest/tonic etc.) from being sent back to OTel itself, thus
    // preventing infinite telemetry generation. The filter levels are set as
    // follows:
    // - Allow `info` level and above by default.
    // - Restrict `opentelemetry`, `hyper`, `tonic`, and `reqwest` completely.
    // Note: This will also drop events from crates like `tonic` etc. even when
    // they are used outside the OTLP Exporter. For more details, see:
    // https://github.com/open-telemetry/opentelemetry-rust/issues/761
    let filter_otel = EnvFilter::new("info")
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("opentelemetry=off".parse().unwrap())
        .add_directive("tonic=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap());
    let log_otel_layer = log_otel_layer.with_filter(filter_otel);

    // Create a new tracing::Fmt layer to print the logs to stdout. It has a
    // default filter of `info` level and above, and `debug` and above for logs
    // from OpenTelemetry crates. The filter levels can be customized as needed.
    let filter_fmt = EnvFilter::new("info").add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = formatting_layer().with_filter(filter_fmt);

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(
            init_tracing_opentelemetry::tracing_subscriber_ext::build_level_filter_layer("")
                .unwrap(),
        )
        .with(log_otel_layer)
        .with(fmt_layer)
        .try_init()
        .unwrap()
}

fn formatting_layer<S>() -> Box<dyn Layer<S> + Send + Sync + 'static>
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
