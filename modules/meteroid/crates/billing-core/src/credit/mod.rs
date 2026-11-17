//! Customer credit on a document: which grants pay how much of it, and how that lands on its
//! lines and totals.

pub mod allocation;
pub mod owed;
pub mod settle;

pub use settle::{Cash, Drawn, Grants, Settlement, TotalBasis, settle};

/// Why a settlement could not be computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettleError {
    /// An amount beyond what minor units in `i64` hold.
    Overflow,
    /// A usage charge names no demand of its request.
    UnknownDemand,
    /// The lines' shares of a draw do not add up to it (I3, I23).
    Unbalanced,
}

impl From<allocation::Overflow> for SettleError {
    fn from(_: allocation::Overflow) -> Self {
        Self::Overflow
    }
}
