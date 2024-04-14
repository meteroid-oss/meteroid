use super::super::errors::ComputeError;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::ops::Mul;

pub trait ToCents {
    fn to_cents(&self) -> Result<i64, ComputeError>;
    fn to_cents_f64(&self) -> Result<f64, ComputeError>;
}

impl ToCents for Decimal {
    fn to_cents(&self) -> Result<i64, ComputeError> {
        let cents = self
            .mul(Decimal::from(100))
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
            .ok_or_else(|| ComputeError::ConversionError)?;

        Ok(cents)
    }

    fn to_cents_f64(&self) -> Result<f64, ComputeError> {
        let cents = self
            .mul(Decimal::from(100))
            .round_dp_with_strategy(6, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_f64()
            .ok_or_else(|| ComputeError::ConversionError)?;

        Ok(cents)
    }
}

pub fn only_positive(price_cents: i64) -> u64 {
    if price_cents > 0 {
        price_cents as u64
    } else {
        0
    }
}
