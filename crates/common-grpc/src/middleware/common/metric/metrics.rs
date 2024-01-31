use common_logging::GLOBAL_METER;
use once_cell::sync::Lazy;
use opentelemetry::metrics::*;
pub use opentelemetry::KeyValue;

pub static CALL_COUNTER: Lazy<Counter<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_counter("grpc.call.counter")
        .with_description("gRPC call")
        .init()
});

pub static CALL_LATENCY: Lazy<Histogram<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_histogram("grpc.call.latency")
        .with_description("gRPC call latency")
        .init()
});
