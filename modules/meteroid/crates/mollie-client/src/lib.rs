//! Minimal Mollie v2 API client (based on https://github.com/mollie/openapi), covering only what
//! the payment connector uses.

pub mod amount;
pub mod chargebacks;
pub mod client;
pub mod customers;
pub mod error;
pub mod mandates;
pub mod methods;
pub mod payments;
mod request;
pub mod webhook;
