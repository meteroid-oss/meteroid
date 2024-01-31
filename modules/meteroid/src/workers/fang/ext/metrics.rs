use once_cell::sync::Lazy;
use opentelemetry::metrics::*;

use common_logging::GLOBAL_METER;

pub static ARCHIVER_MOVED_ROWS_COUNTER: Lazy<Counter<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_counter("fang.archiver.moved_rows")
        .with_description("Amount of rows moved to archive")
        .init()
});

pub static CLEANER_DELETED_ROWS_COUNTER: Lazy<Counter<u64>> = Lazy::new(|| {
    GLOBAL_METER
        .u64_counter("fang.cleaner.deleted_rows")
        .with_description("Amount of rows deleted from archive")
        .init()
});
