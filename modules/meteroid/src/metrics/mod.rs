use common_logging::GLOBAL_METER;
use once_cell::sync::Lazy;
use opentelemetry::metrics::*;

pub static REQUEST_COUNTER: Lazy<Counter<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_counter("request_counter")
        .with_description("")
        .init()
});
