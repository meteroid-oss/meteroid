use once_cell::sync::Lazy;
use opentelemetry::metrics::*;
pub use opentelemetry::KeyValue;
use std::time::Duration;

use common_logging::GLOBAL_METER;

static CALL_COUNTER: Lazy<Counter<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_counter("worker.call.counter")
        .with_description("Amount of calls performed by worker")
        .init()
});

static CALL_LATENCY: Lazy<Histogram<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_histogram("worker.call.latency")
        .with_description("Worker call latency")
        .init()
});

pub fn record_call<S, E>(worker: &str, status: &error_stack::Result<S, E>, duration: Duration) {
    let status_str = match status {
        Err(_) => "error",
        Ok(_) => "ok",
    };

    let attributes = &[
        KeyValue {
            key: "worker".into(),
            value: worker.to_string().into(),
        },
        KeyValue {
            key: "status".into(),
            value: status_str.to_string().into(),
        },
    ];

    CALL_COUNTER.add(1, attributes);
    CALL_LATENCY.record(duration.as_millis() as u64, attributes);
}
