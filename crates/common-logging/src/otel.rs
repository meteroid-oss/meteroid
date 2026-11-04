use init_tracing_opentelemetry::tracing_opentelemetry::OpenTelemetryLayer;
use init_tracing_opentelemetry::{Error, init_propagator, tracing_opentelemetry};
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_appender_tracing::layer::{OpenTelemetryTracingBridge, TracingSpanAttributes};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use std::borrow::Cow;
use std::time::Duration;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer};

use crate::logging::formatting_layer;
use common_config::telemetry::TelemetryConfig;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Link, SpanKind, TraceContextExt, TraceId, TraceState};
use opentelemetry_resource_detectors::{HostResourceDetector, K8sResourceDetector};
use opentelemetry_sdk::trace::{
    SamplingDecision, SamplingResult, SdkTracerProvider, ShouldSample, Tracer,
};
use opentelemetry_sdk::{Resource, resource::ResourceDetector};
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;

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
    let (trace_layer, _guard) = build_tracer_layer_with_resource_and_name(get_resource(), "")
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

    let otel_layer = log_bridge(&logger_provider);
    let filter_otel = crate::logging::quiet_noisy_targets(EnvFilter::new("info"));
    let otel_layer = otel_layer.with_filter(filter_otel);

    tracing_subscriber::registry()
        .with(trace_layer)
        .with(otel_layer)
        .with(crate::logging::quiet_noisy_targets(level_filter_layer()))
        .with(formatting_layer())
        .init();
}

/// Bridges `tracing` events to OTLP log records, carrying the correlation fields of the
/// enclosing spans (see [`crate::context`]).
fn log_bridge<P, L>(provider: &P) -> OpenTelemetryTracingBridge<P, L>
where
    P: opentelemetry::logs::LoggerProvider<Logger = L> + Send + Sync,
    L: opentelemetry::logs::Logger + Send + Sync,
{
    OpenTelemetryTracingBridge::builder(provider)
        .with_tracing_span_attributes(TracingSpanAttributes::allowlist(crate::context::FIELDS))
        .build()
}

// init_tracing_opentelemetry::tracing_subscriber_ext::build_tracer_layer_with_resource is deprecated
// in favor of TracingConfig which in turn doesn't allow to set custom resources so
// Note: copied from init_tracing_opentelemetry::tracing_subscriber_ext::build_tracer_layer_with_resource_and_name
pub(crate) fn build_tracer_layer_with_resource_and_name<S>(
    otel_rsrc: Resource,
    tracer_name: impl Into<Cow<'static, str>>,
) -> Result<(OpenTelemetryLayer<S, Tracer>, SdkTracerProvider), Error>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    // gRPC only: `TelemetryConfig::set_otel_env_vars` pins the traces protocol to it.
    let exporter = SpanExporter::builder()
        .with_tonic()
        .build()
        .map_err(|e| Error::SetupError(e.to_string()))?;
    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(otel_rsrc)
        .with_batch_exporter(exporter)
        .with_sampler(NoRootClientSpans::from_env())
        .build();
    init_propagator()?;
    let layer = tracing_opentelemetry::layer()
        .with_error_records_to_exceptions(true)
        .with_tracer(tracer_provider.tracer(tracer_name));
    global::set_tracer_provider(tracer_provider.clone());
    Ok((layer, tracer_provider))
}

/// `RUST_LOG`, else `OTEL_LOG_LEVEL`, else `info`; as init-tracing-opentelemetry's
/// `build_level_filter_layer`, which needs its HTTP exporter feature.
fn level_filter_layer() -> EnvFilter {
    let directives = std::env::var("RUST_LOG")
        .or_else(|_| std::env::var("OTEL_LOG_LEVEL"))
        .unwrap_or_else(|_| "info".to_string());
    EnvFilter::builder()
        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
        .parse_lossy(directives)
        .add_directive("otel::tracing=trace".parse().expect("a valid directive"))
}

/// Drops client spans that have no parent, so an outgoing call never starts a trace; the
/// unsampled `traceparent` it sends makes a parent-based server drop its side too. Calls made
/// inside a request or unit of work join that trace. Everything else goes to the sampler set by
/// `OTEL_TRACES_SAMPLER*` (parent-based, sampling everything, by default).
#[derive(Clone, Debug)]
struct NoRootClientSpans(Box<dyn ShouldSample>);

impl NoRootClientSpans {
    fn from_env() -> Self {
        Self(opentelemetry_sdk::trace::Config::default().sampler)
    }
}

impl ShouldSample for NoRootClientSpans {
    fn should_sample(
        &self,
        parent_context: Option<&opentelemetry::Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        let has_parent = parent_context.is_some_and(|cx| cx.span().span_context().is_valid());
        if *span_kind == SpanKind::Client && !has_parent {
            return SamplingResult {
                decision: SamplingDecision::Drop,
                attributes: Vec::new(),
                trace_state: TraceState::default(),
            };
        }
        self.0
            .should_sample(parent_context, trace_id, name, span_kind, attributes, links)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Key;
    use opentelemetry::logs::AnyValue;
    use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};

    fn attribute(record: &opentelemetry_sdk::logs::SdkLogRecord, key: &str) -> Option<String> {
        record
            .attributes_iter()
            .find(|(k, _)| *k == Key::from(key.to_owned()))
            .map(|(_, v)| match v {
                AnyValue::String(s) => s.to_string(),
                other => format!("{other:?}"),
            })
    }

    #[test]
    fn log_records_carry_correlation_fields_of_enclosing_spans() {
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(log_bridge(&provider));

        tracing::subscriber::with_default(subscriber, || {
            let outer = tracing::info_span!(
                "request_context",
                tenant_id = "tnt_1",
                customer_id = tracing::field::Empty,
                route = "/x"
            );
            let _outer = outer.enter();
            outer.record("customer_id", "cus_1");
            let no_invoice: Option<&str> = None;
            let inner = tracing::info_span!(
                "work",
                subscription_id = Some("sub_1"),
                invoice_id = no_invoice
            );
            let _inner = inner.enter();
            tracing::info!(attempt = 2, "processing");
        });

        let logs = exporter.get_emitted_logs().unwrap();
        let record = &logs.first().expect("one log record").record;
        assert_eq!(attribute(record, "tenant_id").as_deref(), Some("tnt_1"));
        assert_eq!(attribute(record, "customer_id").as_deref(), Some("cus_1"));
        assert_eq!(
            attribute(record, "subscription_id").as_deref(),
            Some("sub_1")
        );
        assert!(attribute(record, "attempt").is_some());
        assert_eq!(attribute(record, "invoice_id"), None);
        assert_eq!(attribute(record, "route"), None);
    }

    #[test]
    fn log_records_carry_the_trace_of_their_span() {
        use opentelemetry_sdk::trace::InMemorySpanExporter;

        let log_exporter = InMemoryLogExporter::default();
        let logger_provider = SdkLoggerProvider::builder()
            .with_simple_exporter(log_exporter.clone())
            .build();
        let span_exporter = InMemorySpanExporter::default();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(span_exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("test")))
            .with(log_bridge(&logger_provider));

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("request_context", tenant_id = "tnt_1");
            let _span = span.enter();
            tracing::info!("inside the request");
        });

        let spans = span_exporter.get_finished_spans().unwrap();
        let span = spans.first().expect("one span");
        let logs = log_exporter.get_emitted_logs().unwrap();
        let trace = logs
            .first()
            .expect("one log record")
            .record
            .trace_context()
            .expect("log record linked to a trace");
        assert_eq!(trace.trace_id, span.span_context.trace_id());
        assert_eq!(trace.span_id, span.span_context.span_id());
    }

    #[test]
    fn traces_never_start_at_an_outgoing_call() {
        use opentelemetry_sdk::trace::InMemorySpanExporter;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .with_sampler(NoRootClientSpans::from_env())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));

        tracing::subscriber::with_default(subscriber, || {
            let poll = tracing::info_span!("poll", otel.kind = "client");
            let propagated = poll.context().span().span_context().clone();
            assert!(propagated.is_valid() && !propagated.is_sampled());
            drop(poll);
            assert!(exporter.get_finished_spans().unwrap().is_empty());

            let work = tracing::info_span!("work");
            let _work = work.enter();
            let call = tracing::info_span!("call", otel.kind = "client");
            assert!(call.context().span().span_context().is_sampled());
        });

        let names: Vec<_> = exporter
            .get_finished_spans()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["call", "work"]);
    }
}
