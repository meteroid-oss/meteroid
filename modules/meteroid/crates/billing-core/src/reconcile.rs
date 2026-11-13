//! What a fee should have billed for a period against what its documents billed, per charge
//! (R-079, R-080).

use chrono::NaiveDate;
use uuid::Uuid;

use rust_decimal::Decimal;

use crate::DayFraction;
use crate::allocation::largest_remainder;
use crate::charge::{ChargeKey, ChargeKind, Fee};
use crate::issued::IssuedLine;

/// A fee's price in force over `[from, until)`, within the period `period` it bills on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub fee: Fee,
    /// What the fee bills for a whole period at this price.
    pub full: i64,
    pub period: (NaiveDate, NaiveDate),
    pub from: NaiveDate,
    pub until: NaiveDate,
}

impl Term {
    /// The share of the period's price for `[from, until)`: what is left of it at `from` less
    /// what is left at `until`, each rounded once, so the terms tiling a period add up to the
    /// price exactly (R-079).
    pub fn amount(&self) -> i64 {
        let (start, end) = self.period;
        let left = |at: NaiveDate| DayFraction::remaining(start, end, at).prorate(self.full);
        left(self.from.clamp(start, end)) - left(self.until.clamp(start, end))
    }
}

/// What a charge should have billed by now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    pub key: ChargeKey,
    pub amount: i64,
}

/// The charges `terms` add up to, one per fee and period.
pub fn expected(subscription: Option<Uuid>, terms: &[Term]) -> Vec<Expected> {
    let mut charges: Vec<Expected> = Vec::new();
    for term in terms {
        let key = ChargeKey {
            subscription,
            fee: term.fee.clone(),
            from: term.period.0,
            until: term.period.1,
            kind: ChargeKind::Fee,
        };
        match charges.iter_mut().find(|charge| charge.key == key) {
            Some(charge) => charge.amount += term.amount(),
            None => charges.push(Expected {
                key,
                amount: term.amount(),
            }),
        }
    }
    charges
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adjustment {
    /// Billed for the charge on top of what its documents billed.
    Bill { key: ChargeKey, amount: i64 },
    /// Taken back from an issued line.
    Credit { line: String, amount: i64 },
    /// Owed back beyond what the charge's lines have left.
    Short { key: ChargeKey, amount: i64 },
}

/// Which of a charge's lines a credit is taken back from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeBack {
    /// The latest lines first: a fee cut back gives back what was added last.
    LatestFirst,
    /// Each line in proportion to its time left past the date, the rest latest first: a fee
    /// ending there gives back each line's unused time.
    Unused(NaiveDate),
}

/// Whether an issued line billed the charge `key`: the same subscription, fee and kind, over
/// part of the key's period.
fn billed_by(line: &IssuedLine, key: &ChargeKey) -> bool {
    line.key.subscription == key.subscription
        && line.key.fee == key.fee
        && line.key.kind == key.kind
        && key.from <= line.from
        && line.until <= key.until
}

/// For each expected charge, the difference between what it should have billed and what its
/// lines still bill: billed when positive, taken back from its lines as `take_back` says when
/// negative, never more than a line has left after all its notes (R-080). What it billed counts
/// the engine's corrections, not an operator's notes, which are never billed again (R-082).
/// Amounts are in the figures prices are in: a tax-inclusive line is read at its quote (R-081).
/// Charges not in `expected` are left alone.
pub fn reconcile(
    expected: &[Expected],
    issued: &[IssuedLine],
    take_back: TakeBack,
) -> Vec<Adjustment> {
    let mut adjustments = Vec::new();
    for charge in expected {
        let mut lines: Vec<&IssuedLine> = issued
            .iter()
            .filter(|line| billed_by(line, &charge.key))
            .collect();
        let billed: i64 = lines
            .iter()
            .map(|line| line.quoted_of(line.subtotal - line.corrected))
            .sum();
        let delta = charge.amount - billed;
        if delta > 0 {
            adjustments.push(Adjustment::Bill {
                key: charge.key.clone(),
                amount: delta,
            });
            continue;
        }
        lines.sort_by(|a, b| b.from.cmp(&a.from).then_with(|| b.id.cmp(&a.id)));
        let left: Vec<i64> = lines
            .iter()
            .map(|line| line.quoted_of((line.subtotal - line.already_credited).max(0)))
            .collect();
        let mut owed = -delta;
        let mut taken = vec![0; lines.len()];
        if let TakeBack::Unused(at) = take_back {
            let unused: Vec<i64> = lines
                .iter()
                .zip(&left)
                .map(|(line, left)| {
                    if line.from <= at && at < line.until {
                        DayFraction::remaining(line.from, line.until, at).prorate(*left)
                    } else {
                        0
                    }
                })
                .collect();
            let total: i64 = unused.iter().sum();
            if total > 0 {
                let share = owed.min(total);
                let exact: Vec<Decimal> = unused
                    .iter()
                    .map(|unused| {
                        Decimal::from(share) * Decimal::from(*unused) / Decimal::from(total)
                    })
                    .collect();
                taken = largest_remainder(&exact);
                owed -= taken.iter().sum::<i64>();
            }
        }
        for (taken, left) in taken.iter_mut().zip(&left) {
            let more = owed.min(left - *taken).max(0);
            *taken += more;
            owed -= more;
        }
        for (line, amount) in lines.iter().zip(taken) {
            if amount > 0 {
                adjustments.push(Adjustment::Credit {
                    line: line.id.clone(),
                    amount,
                });
            }
        }
        if owed > 0 {
            adjustments.push(Adjustment::Short {
                key: charge.key.clone(),
                amount: owed,
            });
        }
    }
    adjustments
}

/// The signed total of `adjustments` for each fee: billed positive, taken back negative.
pub fn net_by_fee(adjustments: &[Adjustment], issued: &[IssuedLine]) -> Vec<(Fee, i64)> {
    let mut net: Vec<(Fee, i64)> = Vec::new();
    let mut add = |fee: &Fee, amount: i64| match net.iter_mut().find(|(f, _)| f == fee) {
        Some((_, total)) => *total += amount,
        None => net.push((fee.clone(), amount)),
    };
    for adjustment in adjustments {
        match adjustment {
            Adjustment::Bill { key, amount } => add(&key.fee, *amount),
            Adjustment::Short { key, amount } => add(&key.fee, -amount),
            Adjustment::Credit { line, amount } => {
                if let Some(line) = issued.iter().find(|issued| issued.id == *line) {
                    add(&line.key.fee, -amount);
                }
            }
        }
    }
    net
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issued::IssuedKind;
    use proptest::prelude::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + chrono::Days::new(u64::from(d))
    }

    fn period() -> (NaiveDate, NaiveDate) {
        (day(0), day(31))
    }

    fn term(full: i64, from: u32, until: u32) -> Term {
        Term {
            fee: Fee::Component(Uuid::from_u128(1)),
            full,
            period: period(),
            from: day(from),
            until: day(until),
        }
    }

    fn issued(id: &str, key: &ChargeKey, from: u32, subtotal: i64) -> IssuedLine {
        IssuedLine {
            id: id.to_owned(),
            key: ChargeKey {
                from: day(from),
                ..key.clone()
            },
            kind: IssuedKind::Fee {
                price_component: None,
                add_on: false,
            },
            lineage: None,
            quantity: None,
            unit_priced: true,
            from: day(from),
            until: period().1,
            subtotal,
            quoted: None,
            already_credited: 0,
            corrected: 0,
            settled: None,
        }
    }

    #[test]
    fn a_downgrade_takes_back_the_old_price_and_bills_the_new_for_the_time_left() {
        let expected = expected(None, &[term(3_100, 0, 16), term(1_550, 16, 31)]);
        let line = issued("a", &expected[0].key, 0, 3_100);
        let adjustments = reconcile(
            &expected,
            std::slice::from_ref(&line),
            TakeBack::LatestFirst,
        );
        assert_eq!(
            adjustments,
            vec![Adjustment::Credit {
                line: "a".to_owned(),
                amount: 750
            }]
        );
    }

    #[test]
    fn a_removed_fee_is_taken_back_from_its_latest_lines_first() {
        let expected = expected(None, &[term(3_100, 0, 0)]);
        let lines = [
            issued("a", &expected[0].key, 0, 3_100),
            issued("b", &expected[0].key, 10, 1_000),
        ];
        let adjustments = reconcile(&expected, &lines, TakeBack::LatestFirst);
        assert_eq!(
            adjustments,
            vec![
                Adjustment::Credit {
                    line: "b".to_owned(),
                    amount: 1_000
                },
                Adjustment::Credit {
                    line: "a".to_owned(),
                    amount: 3_100
                }
            ]
        );
    }

    /// A fee removed on day 16 owes back 15 days of 31.00 (15.00); an operator already gave back
    /// 30.00 of the line, so only its last 1.00 can be taken back, and nothing is billed again.
    #[test]
    fn an_operator_note_caps_what_is_taken_back_and_is_never_billed_again() {
        let expected = expected(None, &[term(3_100, 0, 16)]);
        let mut line = issued("a", &expected[0].key, 0, 3_100);
        line.already_credited = 3_000;
        assert_eq!(
            reconcile(&expected, &[line], TakeBack::LatestFirst),
            vec![
                Adjustment::Credit {
                    line: "a".to_owned(),
                    amount: 100
                },
                Adjustment::Short {
                    key: expected[0].key.clone(),
                    amount: 1_400
                }
            ]
        );
    }

    /// 10 seats billed for 310.00 on day 0, 10 more on day 10 for 210.00 (21 of 31 days), all
    /// ending on day 16: each line gives back its own 15 days, never the newest line whole.
    #[test]
    fn an_ending_fee_gives_back_each_lines_unused_time() {
        let expected = expected(None, &[term(31_000, 0, 10), term(62_000, 10, 16)]);
        let lines = [
            issued("first", &expected[0].key, 0, 31_000),
            issued("added", &expected[0].key, 10, 21_000),
        ];
        assert_eq!(
            reconcile(&expected, &lines, TakeBack::Unused(day(16))),
            vec![
                Adjustment::Credit {
                    line: "added".to_owned(),
                    amount: 15_000
                },
                Adjustment::Credit {
                    line: "first".to_owned(),
                    amount: 15_000
                }
            ]
        );
    }

    proptest! {
        #[test]
        fn terms_tiling_a_period_add_up_to_its_price(
            full in 0i64..10_000_000,
            mut cuts in prop::collection::vec(1u32..31, 0..6),
        ) {
            cuts.sort();
            cuts.dedup();
            let bounds: Vec<u32> = std::iter::once(0).chain(cuts).chain(std::iter::once(31)).collect();
            let total: i64 = bounds.windows(2).map(|w| term(full, w[0], w[1]).amount()).sum();
            prop_assert_eq!(total, full);
        }

        #[test]
        fn reconciling_what_was_reconciled_changes_nothing(
            full in 1i64..1_000_000,
            new_full in 0i64..1_000_000,
            at in 1u32..31,
            credited in 0i64..1_000_000,
            ending in any::<bool>(),
        ) {
            let take_back = if ending { TakeBack::Unused(day(at)) } else { TakeBack::LatestFirst };
            let expected = expected(None, &[term(full, 0, at), Term { full: new_full, ..term(0, at, 31) }]);
            let mut line = issued("a", &expected[0].key, 0, full);
            line.already_credited = credited.min(full);
            line.corrected = line.already_credited;
            let first = reconcile(&expected, std::slice::from_ref(&line), take_back);
            let mut lines = vec![line];
            for adjustment in &first {
                match adjustment {
                    Adjustment::Credit { amount, .. } => {
                        lines[0].already_credited += amount;
                        lines[0].corrected += amount;
                    }
                    Adjustment::Bill { key, amount } => lines.push(issued("b", key, at, *amount)),
                    Adjustment::Short { .. } => {}
                }
            }
            prop_assert!(reconcile(&expected, &lines, take_back).is_empty());
            for line in &lines {
                prop_assert!(line.already_credited <= line.subtotal);
            }
        }
    }
}
