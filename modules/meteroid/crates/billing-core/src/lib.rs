//! Pure billing computations.
//!
//! Everything here is a function of its arguments: no database, no clock, no network, and no
//! floating-point money. Each function cites the billing rule it implements by ID.

pub mod allocation;
pub mod charge;
pub mod issued;
pub mod proration;
pub mod reconcile;
pub mod rounding;
pub mod totals;

pub use allocation::{largest_remainder, split};
pub use charge::{ChargeKey, ChargeKind, Fee, line_ids};
pub use issued::{IssuedKind, IssuedLine, shortfall_credits};
pub use proration::DayFraction;
pub use reconcile::{Adjustment, Expected, TakeBack, Term, expected, net_by_fee, reconcile};
pub use rounding::{div_round, round_to_minor, scale};
pub use totals::{LineAmounts, Totals, amount_due};
