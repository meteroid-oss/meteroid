use chrono::NaiveDate;

pub mod clients;
mod engine;
mod errors;

pub use engine::invoice::InvoiceEngine;
pub use engine::period::calculate_period_range;
pub use errors::ComputeError;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
    // is_partial: bool, // should prorate
}

#[derive(Debug, Clone)]
pub struct ComponentPeriods {
    pub arrear: Option<Period>,
    pub advance: Option<Period>,
    pub proration_factor: Option<f64>,
}
