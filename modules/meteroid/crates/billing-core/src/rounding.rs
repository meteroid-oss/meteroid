//! The single rounding rule for minor units (R-001): half away from zero.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

/// `numerator / denominator` rounded half away from zero, exactly. Panics on a zero denominator.
pub fn div_round(numerator: i128, denominator: i128) -> i128 {
    assert!(denominator != 0, "div_round by zero");
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if 2 * remainder.abs() >= denominator {
        quotient + numerator.signum()
    } else {
        quotient
    }
}

/// `amount × part / whole`, rounded once: the share of `amount` that `part` is of `whole`.
pub fn scale(amount: i64, part: i64, whole: i64) -> i64 {
    let scaled = div_round(i128::from(amount) * i128::from(part), i128::from(whole));
    i64::try_from(scaled).expect("a scaled amount fits in i64")
}

/// An exact amount in minor units, rounded to a whole minor unit.
pub fn round_to_minor(amount: Decimal) -> i64 {
    amount
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rust_decimal_macros::dec;

    #[test]
    fn halves_round_away_from_zero() {
        assert_eq!(div_round(5, 2), 3);
        assert_eq!(div_round(-5, 2), -3);
        assert_eq!(div_round(4, 3), 1);
        assert_eq!(div_round(-4, 3), -1);
        assert_eq!(div_round(0, 7), 0);
        assert_eq!(round_to_minor(dec!(2.5)), 3);
        assert_eq!(scale(2_000, 1, 3), 667);
        assert_eq!(scale(-2_000, 1, 3), -667);
        assert_eq!(scale(-2_000, -1, -3), -667);
        assert_eq!(div_round(5, -2), -3);
        assert_eq!(round_to_minor(dec!(-2.5)), -3);
    }

    proptest! {
        #[test]
        fn matches_decimal_rounding(n in -1_000_000_000_000i64..1_000_000_000_000, d in 1i64..1_000_000) {
            let exact = Decimal::from(n) / Decimal::from(d);
            prop_assert_eq!(div_round(n.into(), d.into()), i128::from(round_to_minor(exact)));
        }

        #[test]
        fn is_odd_symmetric(n in -1_000_000_000i64..1_000_000_000, d in 1i64..1_000_000) {
            prop_assert_eq!(div_round((-n).into(), d.into()), -div_round(n.into(), d.into()));
        }
    }
}
