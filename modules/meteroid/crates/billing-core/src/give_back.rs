//! What an ending gives back of what was billed in advance besides fees, which reconcile gives
//! back by their unused time (R-077): a pack tied to the period (OQ1), and a shortfall charged up
//! front by the rule before the true-up.

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::DayFraction;
use crate::allocation::largest_remainder;
use crate::credit::allocation::Classification;
use crate::issued::{IssuedKind, IssuedLine};

/// A grant a pack line funded, as the ending's settlement of the usage up to it left it (D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackGrant {
    /// What it granted.
    pub face: i64,
    pub classification: Classification,
    /// What is left of it.
    pub left: i64,
}

/// The fee an ending gives back of a pack tied to the period, out of `billed` (what its line still
/// bills, nothing for a pack that billed no money, as an included one): its unused time, capped
/// by the share of its paid credit left, so spending the credit then ending refunds nothing (OQ1).
/// Promotional credit never caps it; a pack with no paid grant (not paid yet) gives back by time.
pub fn pack(billed: i64, unused_time: DayFraction, grants: &[PackGrant]) -> i64 {
    let billed = billed.max(0);
    let by_time = unused_time.prorate(billed);
    let (left, face) = grants
        .iter()
        .filter(|grant| grant.classification == Classification::Paid)
        .fold((0_i64, 0_i64), |(left, face), grant| {
            (
                left.saturating_add(grant.left.max(0)),
                face.saturating_add(grant.face),
            )
        });
    if face <= 0 {
        return by_time;
    }
    by_time.min(DayFraction::new(left, face).prorate(billed))
}

/// What the shortfalls charged up front for a period covering `at` give back (R-077): the
/// period's days left, prorated once over the lines a product-scoped minimum split it into, never
/// above what a line has left. A line an earlier ending settled gives nothing more.
pub fn shortfalls(at: NaiveDate, lines: &[IssuedLine]) -> Vec<(String, i64)> {
    let current: Vec<&IssuedLine> = lines
        .iter()
        .filter(|line| {
            matches!(line.kind, IssuedKind::Shortfall { up_front: true })
                && line.covers(at)
                && line.settled.is_none()
        })
        .collect();
    let mut periods: Vec<(NaiveDate, NaiveDate)> =
        current.iter().map(|line| (line.from, line.until)).collect();
    periods.sort();
    periods.dedup();
    let mut credits = Vec::new();
    for (from, until) in periods {
        let period: Vec<&&IssuedLine> = current
            .iter()
            .filter(|line| (line.from, line.until) == (from, until))
            .collect();
        let billed: i64 = period.iter().map(|line| line.subtotal).sum();
        if billed <= 0 {
            continue;
        }
        let credited = DayFraction::remaining(from, until, at).prorate(billed);
        let exact: Vec<Decimal> = period
            .iter()
            .map(|line| {
                Decimal::from(credited) * Decimal::from(line.subtotal) / Decimal::from(billed)
            })
            .collect();
        for (line, share) in period.iter().zip(largest_remainder(&exact)) {
            let amount = share.min(line.left());
            if amount > 0 {
                credits.push((line.id.clone(), amount));
            }
        }
    }
    credits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charge::{ChargeKey, ChargeKind, Denomination, Fee};
    use proptest::prelude::*;

    fn paid(face: i64, left: i64) -> PackGrant {
        PackGrant {
            face,
            classification: Classification::Paid,
            left,
        }
    }

    fn bonus(face: i64, left: i64) -> PackGrant {
        PackGrant {
            classification: Classification::Promotional,
            ..paid(face, left)
        }
    }

    /// A 500.00 pack of 500.00 of paid credit and a 50.00 bonus, half its period left.
    /// Rules: OQ1
    #[test]
    fn a_pack_gives_back_its_unused_time_capped_by_its_unused_paid_credit() {
        let half = DayFraction::new(15, 30);
        // Nothing used: the unused time decides.
        assert_eq!(pack(50_000, half, &[paid(50_000, 50_000)]), 25_000);
        // A fifth left: a fifth of the fee, though half the period is left.
        assert_eq!(pack(50_000, half, &[paid(50_000, 10_000)]), 10_000);
        // Everything spent, then the change: nothing back.
        assert_eq!(pack(50_000, half, &[paid(50_000, 0)]), 0);
        // Not paid yet (no grant): only time.
        assert_eq!(pack(50_000, half, &[]), 25_000);
        // Rounding of the cap: 50000 × 1 / 3 = 16666.67.
        assert_eq!(pack(50_000, DayFraction::WHOLE, &[paid(3, 1)]), 16_667);
    }

    /// The bonus never caps the fee, spent or not; a pack billing no money gives nothing back.
    /// Rules: OQ1
    #[test]
    fn promotional_credit_never_caps_a_pack_and_an_unpaid_line_gives_nothing() {
        let half = DayFraction::new(15, 30);
        assert_eq!(
            pack(50_000, half, &[paid(50_000, 10_000), bonus(5_000, 5_000)]),
            10_000
        );
        assert_eq!(
            pack(50_000, half, &[paid(50_000, 50_000), bonus(5_000, 0)]),
            25_000
        );
        // A bonus alone is no payment: by time.
        assert_eq!(pack(50_000, half, &[bonus(5_000, 0)]), 25_000);
        assert_eq!(pack(0, half, &[bonus(500_000, 500_000)]), 0);
        // Two paid grants of one line are read together: 30000 of 60000 left.
        assert_eq!(
            pack(
                50_000,
                DayFraction::WHOLE,
                &[paid(40_000, 10_000), paid(20_000, 20_000)]
            ),
            25_000
        );
        // Settled past zero reads as nothing left.
        assert_eq!(pack(50_000, half, &[paid(50_000, -1)]), 0);
    }

    fn day(d: u64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + chrono::Days::new(d)
    }

    fn shortfall(id: &str, until: u64, subtotal: i64) -> IssuedLine {
        IssuedLine {
            id: id.to_owned(),
            key: ChargeKey {
                subscription: None,
                fee: Fee::Minimum { product: None },
                from: day(0),
                until: day(until),
                kind: ChargeKind::Shortfall,
                denomination: Denomination::Money,
            },
            kind: IssuedKind::Shortfall { up_front: true },
            lineage: None,
            quantity: None,
            unit_priced: false,
            from: day(0),
            until: day(until),
            subtotal,
            quoted: None,
            already_credited: 0,
            corrected: 0,
            settled: None,
            voided: false,
        }
    }

    /// 31.00 charged up front over January, ended on day 16: 3100 × 15/31 = 1500 back.
    /// Rules: R-077
    #[test]
    fn a_shortfall_charged_up_front_gives_back_its_days_left() {
        assert_eq!(
            shortfalls(day(16), &[shortfall("a", 31, 3_100)]),
            vec![("a".to_owned(), 1_500)]
        );
        // On its first day, all of it; past its period, nothing.
        assert_eq!(
            shortfalls(day(0), &[shortfall("a", 31, 3_100)]),
            vec![("a".to_owned(), 3_100)]
        );
        assert!(shortfalls(day(31), &[shortfall("a", 31, 3_100)]).is_empty());
    }

    /// Three products' shares of 0.03 over 30 days, ended on day 15: 3 × 15/30 = 1.5 → 2 is
    /// prorated once and split 1, 1, 0, where each line rounded alone would give back 3.
    /// Rules: R-077
    #[test]
    fn a_split_shortfall_is_prorated_once_over_its_lines() {
        let lines = [
            shortfall("a", 30, 1),
            shortfall("b", 30, 1),
            shortfall("c", 30, 1),
        ];
        assert_eq!(
            shortfalls(day(15), &lines),
            vec![("a".to_owned(), 1), ("b".to_owned(), 1)]
        );
    }

    /// Never above what a line has left; nothing from a line an earlier ending settled, a true-up
    /// (not charged up front), or a line of another kind.
    /// Rules: R-077
    #[test]
    fn a_shortfall_gives_back_only_what_its_line_has_left() {
        let credited = IssuedLine {
            already_credited: 3_000,
            ..shortfall("a", 31, 3_100)
        };
        assert_eq!(
            shortfalls(day(16), &[credited]),
            vec![("a".to_owned(), 100)]
        );
        let settled = IssuedLine {
            settled: Some(0),
            ..shortfall("a", 31, 3_100)
        };
        let true_up = IssuedLine {
            kind: IssuedKind::Shortfall { up_front: false },
            ..shortfall("b", 31, 3_100)
        };
        let fee = IssuedLine {
            kind: IssuedKind::Other,
            ..shortfall("c", 31, 3_100)
        };
        assert!(shortfalls(day(16), &[settled, true_up, fee]).is_empty());
    }

    proptest! {
        /// Rules: OQ1
        #[test]
        fn a_pack_gives_back_neither_more_than_it_bills_nor_than_its_time(
            billed in 0i64..100_000_000,
            (days_left, days) in (1i64..400).prop_flat_map(|days| (0..=days, Just(days))),
            grants in prop::collection::vec(
                (0i64..10_000_000, -1_000i64..10_000_000, any::<bool>()),
                0..4,
            ),
        ) {
            let unused_time = DayFraction::new(days_left, days);
            let grants: Vec<PackGrant> = grants
                .into_iter()
                .map(|(face, left, is_paid)| {
                    let left = left.min(face);
                    if is_paid { paid(face, left) } else { bonus(face, left) }
                })
                .collect();
            let given = pack(billed, unused_time, &grants);
            prop_assert!(given >= 0);
            prop_assert!(given <= billed);
            prop_assert!(given <= unused_time.prorate(billed));
            // More paid credit left never gives back less.
            let fuller: Vec<PackGrant> = grants
                .iter()
                .map(|grant| PackGrant { left: grant.face, ..*grant })
                .collect();
            prop_assert!(pack(billed, unused_time, &fuller) >= given);
        }
    }
}
