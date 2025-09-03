use once_cell::sync::Lazy;
pub use opentelemetry::KeyValue;
use opentelemetry::global::meter;
use opentelemetry::metrics::*;

pub mod logging;
pub mod otel;
pub mod telemetry;
pub mod unwrapper;

pub static GLOBAL_METER: Lazy<Meter> = Lazy::new(|| meter("METEROID"));
