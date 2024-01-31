// note:
//   https://github.com/davidB/tracing-opentelemetry-instrumentation-sdk/issues/109
//   refactor these layers into more generic
//   for further usage in axum too

pub mod common;

#[cfg(feature = "server")]
pub mod server;

pub mod client;
