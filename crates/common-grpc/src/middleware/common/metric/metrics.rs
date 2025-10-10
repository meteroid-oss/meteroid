use common_logging::GLOBAL_METER;
pub use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram};

pub static CALL_COUNTER: std::sync::LazyLock<Counter<u64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .u64_counter("grpc.call.counter")
        .with_description("gRPC call")
        .build()
});

pub static CALL_LATENCY: std::sync::LazyLock<Histogram<u64>> = std::sync::LazyLock::new(|| {
    GLOBAL_METER
        .u64_histogram("grpc.call.latency")
        .with_description("gRPC call latency")
        .build()
});
