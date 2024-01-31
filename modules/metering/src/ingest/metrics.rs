use once_cell::sync::Lazy;
use opentelemetry::metrics::*;

use common_logging::GLOBAL_METER;

pub(super) static INGEST_BATCH_SIZE: Lazy<Histogram<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_histogram("metering.ingest.batch_size")
        .with_description("Size of ingested batch")
        .init()
});

pub(super) static INGESTED_EVENTS_TOTAL: Lazy<Counter<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_counter("metering.ingest.ingested_events_total")
        .with_description("Count of event ingested")
        .init()
});
