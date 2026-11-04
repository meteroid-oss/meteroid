//! Middleware implementations for different frameworks
//!
//! Both axum and tonic middleware accept a key extraction function that
//! determines how to extract the tenant_id from the request. This allows
//! the rate limiter to work with any authentication scheme without depending
//! on specific auth types.

#[cfg(feature = "tonic")]
pub mod tonic;

#[cfg(feature = "axum")]
pub mod axum;
