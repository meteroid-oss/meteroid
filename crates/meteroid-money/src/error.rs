use std::{error, fmt};

/// Standard Error type for this crate.
#[derive(Debug, PartialEq, Clone)]
pub enum MoneyError {
    InvalidCurrency,
    InvalidAmount,
    InvalidRatio,
    /// Returned when arithmetic or comparison is attempted between different currencies.
    CurrencyMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    /// Returned when division by zero is attempted.
    DivisionByZero,
    /// Returned when an arithmetic operation overflows.
    Overflow,
    /// Returned when converting to FastMoney would lose precision.
    PrecisionLoss,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MoneyError::InvalidCurrency => write!(f, "Currency was not valid"),
            MoneyError::InvalidAmount => write!(f, "Amount not parsable"),
            MoneyError::InvalidRatio => write!(f, "Ratio was not valid"),
            MoneyError::CurrencyMismatch { expected, actual } => {
                write!(
                    f,
                    "Currency mismatch: expected {}, got {}",
                    expected, actual
                )
            }
            MoneyError::DivisionByZero => write!(f, "Division by zero"),
            MoneyError::Overflow => write!(f, "Arithmetic overflow"),
            MoneyError::PrecisionLoss => write!(f, "Conversion would lose precision"),
        }
    }
}

impl error::Error for MoneyError {
    fn description(&self) -> &str {
        match self {
            MoneyError::InvalidCurrency => "Currency was not valid",
            MoneyError::InvalidAmount => "Amount not parsable",
            MoneyError::InvalidRatio => "Ratio was not valid",
            MoneyError::CurrencyMismatch { .. } => "Currency mismatch",
            MoneyError::DivisionByZero => "Division by zero",
            MoneyError::Overflow => "Arithmetic overflow",
            MoneyError::PrecisionLoss => "Conversion would lose precision",
        }
    }
}

impl From<std::num::ParseIntError> for MoneyError {
    fn from(_err: std::num::ParseIntError) -> MoneyError {
        MoneyError::InvalidAmount
    }
}
