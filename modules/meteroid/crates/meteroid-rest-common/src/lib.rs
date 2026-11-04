//! The REST pieces the core REST API and the modules share: errors, timestamps, query
//! extraction, the shared models and the delegated-credentials trait.

pub mod client;
pub mod currency;
pub mod customproperties;
pub mod datetime;
pub mod delegated_auth;
pub mod entitlements;
pub mod error;
pub mod model;
pub mod query;
pub mod spec;
pub mod webhook_catalog;

pub use query::{QueryParams, empty_string_as_none};
