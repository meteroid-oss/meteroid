//! HTTP client for the GoCardless Pro API. Each resource module exposes a trait
//! implemented on `GoCardlessClient`. Only the
//! `meteroid-store::adapters::payment::gocardless` adapter should use this crate.

pub mod billing_requests;
pub mod client;
pub mod customers;
pub mod error;
pub mod mandates;
pub mod payments;
pub mod request;
pub mod webhook;
