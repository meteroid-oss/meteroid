//! HTTP client for the GoCardless Pro API.
//!
//! Design mirrors `stripe-client`: a `GoCardlessClient` wraps a pooled
//! `reqwest::Client`, plus per-call auth (Bearer access_token) and
//! idempotency. Each resource module exposes a trait (`CustomerApi`,
//! `BillingRequestApi`, …) implemented on `GoCardlessClient`.
//!
//! The adapter in `meteroid-store::adapters::payment::gocardless` consumes
//! these traits; nothing else in the codebase should reach into this crate.

pub mod billing_requests;
pub mod client;
pub mod customers;
pub mod error;
pub mod mandates;
pub mod payments;
pub mod request;
pub mod webhook;
