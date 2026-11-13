//! Splitting an amount into whole minor units that add up (R-003): largest remainder.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::rounding::round_to_minor;

/// Rounds exact shares (in minor units) to whole units whose sum is the rounded sum of the
/// shares: floor every share, then give the leftover units to the largest remainders as Decimal
/// computes them, ties to the earliest share.
pub fn largest_remainder(exact: &[Decimal]) -> Vec<i64> {
    let total = round_to_minor(exact.iter().sum());
    let mut rounded: Vec<i64> = exact
        .iter()
        .map(|share| share.floor().to_i64().unwrap_or(0))
        .collect();

    let leftover = (total - rounded.iter().sum::<i64>()).clamp(0, exact.len() as i64);
    let mut by_remainder: Vec<usize> = (0..exact.len()).collect();
    by_remainder.sort_by(|a, b| {
        let remainder = |i: usize| exact[i] - exact[i].floor();
        remainder(*b).cmp(&remainder(*a)).then_with(|| a.cmp(b))
    });

    for &share in by_remainder.iter().take(leftover as usize) {
        rounded[share] += 1;
    }
    rounded
}

/// Splits `total` minor units in proportion to `weights`, so the parts add up to `total`.
///
/// Without a usable weighting (fewer than two weights, a non-positive sum or a negative weight)
/// the whole amount goes to the first part. A negative total is not the mirror of a positive one:
/// flooring moves its odd units to the last parts.
pub fn split(total: i64, weights: &[Decimal]) -> Vec<i64> {
    let sum: Decimal = weights.iter().sum();
    if weights.len() <= 1 || sum <= Decimal::ZERO || weights.iter().any(|w| *w < Decimal::ZERO) {
        let mut parts = vec![0; weights.len()];
        if let Some(first) = parts.first_mut() {
            *first = total;
        }
        return parts;
    }
    let exact: Vec<Decimal> = weights
        .iter()
        .map(|weight| Decimal::from(total) * weight / sum)
        .collect();
    largest_remainder(&exact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rust_decimal_macros::dec;

    #[test]
    fn favours_the_largest_remainder() {
        assert_eq!(
            largest_remainder(&[dec!(14.44), dec!(129.20)]),
            vec![15, 129]
        );
        assert_eq!(
            largest_remainder(&[dec!(0.5), dec!(0.5), dec!(0.5)]),
            vec![1, 1, 0]
        );
        assert_eq!(split(100, &[dec!(1), dec!(1), dec!(1)]), vec![34, 33, 33]);
        assert_eq!(
            split(-100, &[dec!(1), dec!(1), dec!(1)]),
            vec![-33, -33, -34]
        );
        assert_eq!(split(7, &[dec!(0), dec!(0)]), vec![7, 0]);
        assert_eq!(split(7, &[dec!(1), dec!(-1)]), vec![7, 0]);
        assert_eq!(split(7, &[dec!(5)]), vec![7]);
        assert_eq!(split(7, &[]), Vec::<i64>::new());
        assert_eq!(split(100, &[dec!(0.0001), dec!(0.0002)]), vec![33, 67]);
    }

    #[test]
    fn ties_break_on_the_remainders_decimal_computes() {
        // 80/6 keeps one fewer fractional digit than 20/6, so the exact three-way tie is not one.
        assert_eq!(split(20, &[dec!(4), dec!(1), dec!(1)]), vec![13, 4, 3]);
    }

    fn weights() -> impl Strategy<Value = Vec<Decimal>> {
        prop::collection::vec(prop_oneof![1 => Just(0u32), 3 => 0u32..1_000_000], 2..12)
            .prop_map(|ws| ws.into_iter().map(Decimal::from).collect())
            .prop_filter("a positive sum", |ws: &Vec<Decimal>| {
                ws.iter().sum::<Decimal>() > Decimal::ZERO
            })
    }

    proptest! {
        #[test]
        fn parts_add_up(total in -10_000_000_000i64..10_000_000_000, ws in weights()) {
            prop_assert_eq!(split(total, &ws).iter().sum::<i64>(), total);
        }

        #[test]
        fn each_part_is_within_a_unit_of_its_share(total in -10_000_000_000i64..10_000_000_000, ws in weights()) {
            let sum: Decimal = ws.iter().sum();
            for (part, w) in split(total, &ws).iter().zip(&ws) {
                let exact = Decimal::from(total) * w / sum;
                prop_assert!((Decimal::from(*part) - exact).abs() < Decimal::ONE);
            }
        }

        #[test]
        fn zero_weight_gets_nothing(total in -10_000_000_000i64..10_000_000_000, ws in weights()) {
            for (part, w) in split(total, &ws).iter().zip(&ws) {
                if w.is_zero() {
                    prop_assert_eq!(*part, 0);
                }
            }
        }
    }
}
