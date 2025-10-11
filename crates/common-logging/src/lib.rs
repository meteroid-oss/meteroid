pub use opentelemetry::KeyValue;
use opentelemetry::global::meter;
use opentelemetry::metrics::Meter;

pub mod logging;
pub mod otel;
pub mod telemetry;
pub mod unwrapper;

pub static GLOBAL_METER: std::sync::LazyLock<Meter> =
    std::sync::LazyLock::new(|| meter("METEROID"));
