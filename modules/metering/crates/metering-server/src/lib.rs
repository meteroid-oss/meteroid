//! gRPC composition root of the metering planes; editions add their services through
//! `server::ExtraRoutes`.

pub mod query;
pub mod server;
