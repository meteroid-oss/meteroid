use opentelemetry::metrics::{Counter, Histogram};

use common_logging::GLOBAL_METER;

pub(super) static INGEST_BATCH_SIZE: std::sync::LazyLock<Histogram<u64>> =
    std::sync::LazyLock::new(|| {
        GLOBAL_METER
            .u64_histogram("metering.ingest.batch_size")
            .with_description("Size of ingested batch")
            .build()
    });

pub(super) static INGESTED_EVENTS_TOTAL: std::sync::LazyLock<Counter<u64>> =
    std::sync::LazyLock::new(|| {
        GLOBAL_METER
            .u64_counter("metering.ingest.ingested_events_total")
            .with_description("Count of event ingested")
            .build()
    });
