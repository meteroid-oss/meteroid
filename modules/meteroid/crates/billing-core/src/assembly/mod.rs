//! A document's lines put together: their charge keys, its coupons and their spread, and the
//! netting of tax-inclusive prices.

pub mod charges;
pub mod credit_units;
pub mod discount;
pub mod document;
pub mod tax_inclusive;

pub use document::{Assembled, AssemblyInputs, assemble, net_discount};
