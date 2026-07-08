// Vendored from https://github.com/franzos/world-tax-rs (see LICENSE). Kept close
// to upstream, so it is not held to this workspace's clippy/lint bar.
#![allow(clippy::all)]
#![allow(dead_code)]

pub mod calculation;
mod calculation_test;
pub mod errors;
pub mod provider;
pub mod types;

pub use provider::TaxDatabase;
pub use types::{
    Region, TaxCalculationType, TaxRate, TaxScenario, TaxType, TradeAgreement,
    TradeAgreementOverride, TransactionType, VatRate,
};

pub use errors::{DatabaseError, InputValidationError, ProcessingError};
