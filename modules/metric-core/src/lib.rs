//! Engine-agnostic metric kernel shared by `metering` and `analytics`.

pub mod backend;
pub mod config;
pub mod domain;
pub mod errors;
pub mod json;
pub mod metric;
pub mod migrate;
pub mod sql;
#[cfg(any(test, feature = "test-corpus"))]
pub mod testing;
pub mod utils;
