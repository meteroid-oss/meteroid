use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use std::ops::Mul;

pub trait ToCent {
    fn to_cents(&self) -> Option<i64>;
    fn to_cents_f64(&self) -> Option<f64>;
}

impl ToCent for Decimal {
    fn to_cents(&self) -> Option<i64> {
        self.mul(Decimal::from(100))
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
    }

    fn to_cents_f64(&self) -> Option<f64> {
        self.mul(Decimal::from(100))
            .round_dp_with_strategy(6, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_f64()
    }
}
