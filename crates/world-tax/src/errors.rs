//! Error types and handling.
//!
//! This module defines the various error types used throughout the tax calculation
//! system. These errors are used to represent different failure scenarios that can
//! occur during input validation, database operations, and processing of tax calculations.
//!
//! The errors are categorized into three main types:
//!
//! - `InputValidationError`: Errors related to invalid input data, such as incorrect
//!   country or region codes.
//! - `DatabaseError`: Errors that occur during database operations, such as missing
//!   trade agreements or tax rates.
//! - `ProcessingError`: Errors that occur during the processing of tax calculations,
//!   such as invalid amounts or errors propagated from other error types.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
pub enum InputValidationError {
    #[error("Invalid country code: {0}")]
    InvalidCountryCode(String),
    #[error("Invalid region code: {0}")]
    InvalidRegionCode(String),
    #[error("Unexpected region code: {0} - Country has no regions.")]
    UnexpectedRegionCode(String),
}

#[derive(Debug, Error, Serialize)]
pub enum DatabaseError {
    #[error("Trade agreement not found: {0}")]
    TradeAgreementNotFound(String),
    #[error("Country not found: {0}")]
    CountryNotFound(String),
    #[error("Region not found: {0}")]
    RegionNotFound(String),
    #[error("VAT rate not found: {0}")]
    VatRateNotFound(String),
}

#[derive(Debug, Error, Serialize)]
pub enum ProcessingError {
    #[error("Invalid input: {0}")]
    InputValidationError(InputValidationError),
    #[error("Database error: {0}")]
    DatabaseError(DatabaseError),
    #[error("Invalid amount")]
    InvalidAmount,
}

impl From<InputValidationError> for ProcessingError {
    fn from(err: InputValidationError) -> Self {
        ProcessingError::InputValidationError(err)
    }
}

impl From<DatabaseError> for ProcessingError {
    fn from(err: DatabaseError) -> Self {
        ProcessingError::DatabaseError(err)
    }
}
