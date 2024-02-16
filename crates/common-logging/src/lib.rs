use once_cell::sync::Lazy;
use opentelemetry::global::meter;
use opentelemetry::metrics::*;
pub use opentelemetry::KeyValue;

pub mod init;
mod init_metrics;
pub mod unwrapper;

pub static GLOBAL_METER: Lazy<Meter> = Lazy::new(|| meter("METEROID"));
