//! Subscription integration tests using the new harness.
//!
//! This module contains refactored tests from:
//! - test_trials.rs
//! - test_oncheckout.rs
//! - test_subscription_lifecycle.rs
//!
//! Tests are organized by concern and use rstest for parameterization.

mod activation;
mod checkout;
mod lifecycle;
mod trials;
