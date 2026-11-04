//! Outbound HTTP client for tenant- and admin-supplied destinations: no redirects, http(s)
//! only, no credentials in the URL, an optional CONNECT proxy, and an optional
//! [`DestinationPolicy`] restricting where requests may go.

mod client;
mod error;
mod policy;
mod url_rules;

pub use client::{EgressClient, EgressClientConfig, EgressResponse};
pub use error::EgressError;
pub use policy::{DestinationPolicy, Lookup};
pub use reqwest::Url;
