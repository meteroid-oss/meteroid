//! Pure billing computations.
//!
//! Everything here is a function of its arguments: no database, no clock, no network, and no
//! floating-point money. Each function cites the billing rule it implements by ID.

pub mod allocation;
pub mod proration;
pub mod rounding;

pub use allocation::{largest_remainder, split};
pub use proration::DayFraction;
pub use rounding::{div_round, round_to_minor, scale};
