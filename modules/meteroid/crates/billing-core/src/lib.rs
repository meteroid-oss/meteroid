//! Pure billing computations.
//!
//! Everything here is a function of its arguments: no database, no clock, no network, and no
//! floating-point money. Each function cites the billing rule it implements by ID.

pub mod allocation;
pub mod assembly;
pub mod charge;
pub mod credit;
pub mod credit_note;
pub mod document;
pub mod evaluate;
pub mod facts;
pub mod give_back;
pub mod issued;
pub mod merge;
pub mod minimum;
pub mod pricer;
pub mod pricing;
pub mod proration;
pub mod rate_card;
pub mod reconcile;
pub mod rounding;
pub mod settlement;
#[cfg(test)]
mod stories;
pub mod tax;
pub mod timeline;
pub mod totals;
pub mod transition;

pub use allocation::{allocate_covered, largest_remainder, split};
pub use charge::{ChargeKey, ChargeKind, Denomination, Fee, line_ids};
pub use evaluate::{Charge, DerivedState, DocumentSlot, Evaluation, Status, UsageNeed, evaluate};
pub use facts::{DatedFact, FactRef, Facts};
pub use issued::{IssuedKind, IssuedLine};
pub use pricer::{CreditGrant, Pricer, Rate, UsageReads};
pub use pricing::PricingError;
pub use proration::{DayFraction, DaySpan};
pub use reconcile::{Adjustment, Expected, TakeBack, Term, expected, net_by_fee, reconcile};
pub use rounding::{div_round, round_to_minor, scale};
pub use totals::{LineAmounts, Totals, amount_due};
pub use transition::{Deadlines, Due, Step, Stored, Transition};
