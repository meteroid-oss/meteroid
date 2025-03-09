use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::ops::Mul;

pub trait ToSubunit {
    fn to_subunit_opt(&self, precision: u8) -> Option<i64>;
}

impl ToSubunit for Decimal {
    fn to_subunit_opt(&self, precision: u8) -> Option<i64> {
        self.mul(Decimal::from(10i64.pow(precision as u32)))
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
    }
}

pub trait ToUnit {
    fn to_unit(&self, precision: u8) -> Decimal;
}

impl ToUnit for i64 {
    fn to_unit(&self, precision: u8) -> Decimal {
        Decimal::from(*self) / Decimal::from(10i64.pow(precision as u32))
    }
}
