use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

pub fn formatting_layer<S>() -> Box<dyn Layer<S> + Send + Sync + 'static>
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

/// Demotes known-noisy third-party targets, e.g. the per-request
/// "can not set parent trace_id to span error=SpanDisabled" warning emitted
/// by axum-tracing-opentelemetry when otel tracing is disabled or sampled out.
pub(crate) fn quiet_noisy_targets(filter: EnvFilter) -> EnvFilter {
    filter.add_directive(
        "axum_tracing_opentelemetry=error"
            .parse()
            .expect("invalid log directive"),
    )
}

pub fn init_regular_logging() {
    let filter = quiet_noisy_targets(
        EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .from_env_lossy(),
    );

    tracing_subscriber::registry()
        .with(formatting_layer())
        .with(filter)
        .init();
}
