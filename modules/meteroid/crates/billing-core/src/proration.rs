//! Day-based proration (R-002): an amount for a whole period, scaled by the share of its days.

use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

/// A share of a period as whole days over whole days. Amounts are prorated as
/// `amount × num / den` in Decimal, rounded half away from zero to the minor unit like every
/// invoice line, so a true half-cent is never flipped by an inexact factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayFraction {
    num: i64,
    den: i64,
}

impl DayFraction {
    pub const WHOLE: DayFraction = DayFraction { num: 1, den: 1 };
    pub const NONE: DayFraction = DayFraction { num: 0, den: 1 };

    /// `num / den` clamped to [0, 1]; an empty period is none of it.
    pub fn new(num: i64, den: i64) -> Self {
        if den <= 0 {
            return Self::NONE;
        }
        DayFraction {
            num: num.clamp(0, den),
            den,
        }
    }

    /// What is left of `[period_start, period_end)` from `at` on.
    pub fn remaining(period_start: NaiveDate, period_end: NaiveDate, at: NaiveDate) -> Self {
        Self::new(
            (period_end - at).num_days(),
            (period_end - period_start).num_days(),
        )
    }

    pub fn is_whole(&self) -> bool {
        self.num == self.den
    }

    pub fn prorate(&self, amount_minor: i64) -> i64 {
        (Decimal::from(amount_minor) * Decimal::from(self.num) / Decimal::from(self.den))
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
            .unwrap_or(amount_minor)
    }

    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// `amount × num / den`, multiplied before dividing so a whole-day share of a price is exact
    /// before the caller rounds it (1/28 first would truncate to 28 digits: 0.135 read 0.1349…).
    pub fn prorate_decimal(&self, amount: Decimal) -> Decimal {
        amount * Decimal::from(self.num) / Decimal::from(self.den)
    }

    /// This share of `other`'s share, still exact in whole days.
    pub fn of(self, other: DayFraction) -> Self {
        match (
            self.num.checked_mul(other.num),
            self.den.checked_mul(other.den),
        ) {
            (Some(num), Some(den)) => Self::new(num, den),
            _ => Self::new(
                other
                    .prorate_decimal(Decimal::from(self.num))
                    .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
                    .to_i64()
                    .unwrap_or(self.num),
                self.den,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rounding::scale;
    use proptest::prelude::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn prorates_what_is_left_of_the_period() {
        let rest = DayFraction::remaining(date(2024, 1, 1), date(2024, 2, 1), date(2024, 1, 16));
        assert_eq!(rest.prorate(2_900), 1_497);
        assert_eq!(rest.prorate(-2_900), -1_497);
        assert!(
            DayFraction::remaining(date(2024, 1, 1), date(2024, 2, 1), date(2024, 1, 1)).is_whole()
        );
    }

    #[test]
    fn clamps_to_a_share_of_the_period() {
        assert_eq!(
            DayFraction::new(4, 3),
            DayFraction::WHOLE.of(DayFraction::new(3, 3))
        );
        assert_eq!(DayFraction::new(-1, 3).prorate(900), 0);
        assert_eq!(DayFraction::new(1, 0), DayFraction::NONE);
        assert_eq!(
            DayFraction::new(1, 2)
                .of(DayFraction::new(1, 3))
                .prorate(600),
            100
        );
    }

    fn fraction() -> impl Strategy<Value = DayFraction> {
        (1i64..=366).prop_flat_map(|den| (0..=den).prop_map(move |num| DayFraction::new(num, den)))
    }

    proptest! {
        #[test]
        fn rounds_like_exact_integer_division((num, den) in (1i64..=366).prop_flat_map(|den| (0..=den, Just(den))), amount in -10_000_000_000i64..10_000_000_000) {
            prop_assert_eq!(DayFraction::new(num, den).prorate(amount), scale(amount, num, den));
        }

        #[test]
        fn a_fraction_of_a_fraction_multiplies_exactly(
            (a, b) in (1i64..=366).prop_flat_map(|b| (0..=b, Just(b))),
            (c, d) in (1i64..=366).prop_flat_map(|d| (0..=d, Just(d))),
            amount in -10_000_000_000i64..10_000_000_000,
        ) {
            let nested = DayFraction::new(a, b).of(DayFraction::new(c, d));
            prop_assert_eq!(nested.prorate(amount), scale(amount, a * c, b * d));
        }

        #[test]
        fn never_exceeds_the_whole(f in fraction(), amount in -10_000_000_000i64..10_000_000_000) {
            prop_assert!(f.prorate(amount).abs() <= amount.abs());
        }

        #[test]
        fn credit_mirrors_charge(f in fraction(), amount in -10_000_000_000i64..10_000_000_000) {
            prop_assert_eq!(f.prorate(-amount), -f.prorate(amount));
        }

        #[test]
        fn grows_with_days(den in 1i64..=366, amount in 0i64..10_000_000_000, a in 0i64..=366, b in 0i64..=366) {
            let lo = DayFraction::new(a.min(b), den);
            let hi = DayFraction::new(a.max(b), den);
            prop_assert!(lo.prorate(amount) <= hi.prorate(amount));
        }

        #[test]
        fn whole_and_none_are_exact(amount in -10_000_000_000i64..10_000_000_000) {
            prop_assert_eq!(DayFraction::WHOLE.prorate(amount), amount);
            prop_assert_eq!(DayFraction::NONE.prorate(amount), 0);
        }
    }
}
