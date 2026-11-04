//! The database schema as `diesel print-schema` writes it (`diesel.toml`, never edited by hand)
//! and the Rust enums of its Postgres enum types. Its own crate so that query code changes do not
//! recompile the table macros; the enum derives must sit next to the SQL types they implement.
#![recursion_limit = "256"]

pub mod enums;
pub mod schema;
mod table_ids;
mod xid8;
