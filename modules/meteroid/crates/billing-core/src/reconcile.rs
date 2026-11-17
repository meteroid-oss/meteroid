//! What a fee should have billed for a period against what its documents billed, per charge
//! (R-079, R-080).

use chrono::NaiveDate;
use common_domain::ids::SubscriptionId;

use rust_decimal::Decimal;

use crate::DayFraction;
use crate::allocation::largest_remainder;
use crate::charge::{ChargeKey, ChargeKind, Denomination, Fee};
use crate::issued::IssuedLine;
use crate::proration::DaySpan;

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
        DaySpan {
            period: self.period,
            from: self.from,
            until: self.until,
        }
        .prorate(self.full)
    }
}

/// What a charge should have billed by now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expected {
    pub key: ChargeKey,
    pub amount: i64,
}

/// The charges `terms` add up to, one per fee and period.
pub fn expected(subscription: Option<SubscriptionId>, terms: &[Term]) -> Vec<Expected> {
    let mut charges: Vec<Expected> = Vec::new();
    for term in terms {
        let key = ChargeKey {
            subscription,
            fee: term.fee.clone(),
            from: term.period.0,
            until: term.period.1,
            kind: ChargeKind::Fee,
            denomination: Denomination::Money,
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

/// For each expected charge, the difference between what it should have billed and what its
/// lines still bill: billed when positive, taken back from its lines as `take_back` says when
/// negative, never more than a line has left after all its notes (R-080). What it billed counts
/// the engine's corrections, not an operator's notes, which are never billed again (R-082).
/// Amounts are in the figures prices are in: a tax-inclusive line is read at its quote (R-081).
/// Charges not in `expected` are left alone. A voided line counts as billed, never billed again;
/// what is owed back is taken as if it stood, and what falls on it is forgiven with its document,
/// so live lines give back only their own share (R-096).
pub fn reconcile(
    expected: &[Expected],
    issued: &[IssuedLine],
    take_back: TakeBack,
) -> Vec<Adjustment> {
    let mut adjustments = Vec::new();
    for charge in expected {
        let mut lines: Vec<&IssuedLine> = issued
            .iter()
            .filter(|line| line.bills(&charge.key))
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
        // A voided line gives back as if it stood, its void's own note aside; what falls on it
        // is forgiven with it, owed back to no one. The split is read from before the engine's own
        // take-backs, so a later pass finds it made, a line's unused time never taken twice.
        let left: Vec<i64> = lines
            .iter()
            .map(|line| {
                let credited = if line.voided {
                    line.corrected
                } else {
                    line.already_credited
                };
                line.quoted_of((line.subtotal - credited).max(0))
            })
            .collect();
        let mut owed = -delta;
        let taken = {
            let before: Vec<i64> = lines
                .iter()
                .map(|line| {
                    let credited = if line.voided {
                        line.corrected
                    } else {
                        line.already_credited - line.corrected
                    };
                    line.quoted_of((line.subtotal - credited).max(0))
                })
                .collect();
            let earlier: Vec<i64> = lines
                .iter()
                .map(|line| {
                    if line.voided {
                        0
                    } else {
                        line.quoted_of(line.corrected)
                    }
                })
                .collect();
            let shares = split(
                &lines,
                &before,
                owed + earlier.iter().sum::<i64>(),
                take_back,
            );
            let mut taken = vec![0; lines.len()];
            for (i, line) in lines.iter().enumerate() {
                let rest = (shares[i] - earlier[i]).min(left[i]).max(0);
                let more = if line.voided { shares[i] } else { rest }.min(owed);
                taken[i] = more;
                owed -= more;
            }
            for (i, _) in lines.iter().enumerate().filter(|(_, line)| !line.voided) {
                let more = owed.min(left[i] - taken[i]).max(0);
                taken[i] += more;
                owed -= more;
            }
            taken
        };
        for (line, amount) in lines.iter().zip(taken) {
            if amount > 0 && !line.voided {
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

/// `owed` split over `lines` (latest first), never more than a line's `left`: by each line's
/// time left past the date for an ending, the rest latest first.
fn split(lines: &[&IssuedLine], left: &[i64], mut owed: i64, take_back: TakeBack) -> Vec<i64> {
    let mut taken = vec![0; lines.len()];
    if let TakeBack::Unused(at) = take_back {
        let unused: Vec<i64> = lines
            .iter()
            .zip(left)
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
                .map(|unused| Decimal::from(share) * Decimal::from(*unused) / Decimal::from(total))
                .collect();
            taken = largest_remainder(&exact);
            owed -= taken.iter().sum::<i64>();
        }
    }
    for (taken, left) in taken.iter_mut().zip(left) {
        let more = owed.min(left - *taken).max(0);
        *taken += more;
        owed -= more;
    }
    taken
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
    use uuid::Uuid;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + chrono::Days::new(u64::from(d))
    }

    fn period() -> (NaiveDate, NaiveDate) {
        (day(0), day(31))
    }

    fn term(full: i64, from: u32, until: u32) -> Term {
        Term {
            fee: Fee::Component(Uuid::from_u128(1).into()),
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
            voided: false,
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

    /// Usage up to an ending on day 31, a boundary, and the ending's morning that day: each line
    /// bills its own key, the morning's never the period's.
    /// Rules: R-016, R-080
    #[test]
    fn a_line_of_no_days_bills_only_the_key_of_its_day() {
        let key = |from: u32, until: u32| ChargeKey {
            subscription: None,
            fee: Fee::Component(Uuid::from_u128(1).into()),
            from: day(from),
            until: day(until),
            kind: ChargeKind::Usage {
                metric: Uuid::from_u128(3).into(),
                dimensions: std::collections::BTreeMap::new(),
            },
            denomination: Denomination::Money,
        };
        let line = |id: &str, from: u32, until: u32, subtotal: i64| IssuedLine {
            until: day(until),
            ..issued(id, &key(from, until), from, subtotal)
        };
        let lines = [line("period", 0, 31, 700), line("morning", 31, 31, 200)];
        let period = Expected {
            key: key(0, 31),
            amount: 700,
        };
        let morning = Expected {
            key: key(31, 31),
            amount: 300,
        };
        assert!(reconcile(std::slice::from_ref(&period), &lines, TakeBack::LatestFirst).is_empty());
        assert_eq!(
            reconcile(&[period, morning], &lines, TakeBack::LatestFirst),
            vec![Adjustment::Bill {
                key: key(31, 31),
                amount: 100
            }]
        );
    }

    /// January at 31.00, its line voided. Owed whole or cut to 16 of 31 days (3100 × 16/31 = 1600),
    /// the void forgave it all: nothing billed, nothing taken back, nothing owed back.
    /// Rules: R-096
    #[test]
    fn a_void_forgives_what_its_line_billed() {
        let whole = expected(None, &[term(3_100, 0, 31)]);
        let voided = IssuedLine {
            voided: true,
            ..issued("voided", &whole[0].key, 0, 3_100)
        };
        assert!(reconcile(&whole, std::slice::from_ref(&voided), TakeBack::LatestFirst).is_empty());
        let cut = expected(None, &[term(3_100, 0, 16)]);
        assert_eq!(cut[0].amount, 1_600);
        assert!(
            reconcile(
                &cut,
                std::slice::from_ref(&voided),
                TakeBack::Unused(day(16))
            )
            .is_empty()
        );
    }

    /// Ten seats for 31.00 voided, five more for the 15 days from day 16: the step adds
    /// 1550 × 15/31 = 750, which no void forgave: billed, never the voided 3100 again.
    /// Rules: R-096
    #[test]
    fn a_void_forgives_only_what_its_line_billed() {
        let owed = expected(None, &[term(3_100, 0, 16), term(4_650, 16, 31)]);
        assert_eq!(owed[0].amount, 3_100 + 750);
        let voided = IssuedLine {
            voided: true,
            ..issued("voided", &owed[0].key, 0, 3_100)
        };
        assert_eq!(
            reconcile(&owed, std::slice::from_ref(&voided), TakeBack::LatestFirst),
            vec![Adjustment::Bill {
                key: owed[0].key.clone(),
                amount: 750
            }]
        );
    }

    /// A voided 31.00 and a live 10.00 on one charge owing 31.00: the void forgave the 31.00, so
    /// the live line billed 10.00 too many and gives it back; owing 41.00, both stand. Owing
    /// 16.00 after an ending, only the live 10.00 can be given back: no more is owed back.
    /// Rules: R-096
    #[test]
    fn a_voided_line_counts_as_billed_and_only_live_lines_give_back() {
        let whole = expected(None, &[term(3_100, 0, 31)]);
        let voided = IssuedLine {
            voided: true,
            ..issued("voided", &whole[0].key, 0, 3_100)
        };
        let live = issued("live", &whole[0].key, 10, 1_000);
        let lines = [voided, live];
        assert_eq!(
            reconcile(&whole, &lines, TakeBack::LatestFirst),
            vec![Adjustment::Credit {
                line: "live".to_owned(),
                amount: 1_000
            }]
        );
        let more = expected(None, &[term(4_100, 0, 31)]);
        assert!(reconcile(&more, &lines, TakeBack::LatestFirst).is_empty());
        let cut = expected(None, &[term(3_100, 0, 16)]);
        assert_eq!(
            reconcile(&cut, &lines, TakeBack::Unused(day(16))),
            vec![Adjustment::Credit {
                line: "live".to_owned(),
                amount: 1_000
            }]
        );
    }

    /// Ten seats for 31.00 voided, five more from day 10 live (1050), ended on day 16: the void
    /// forgives its own seats (1500 unused), the step gives back its own 15 of 21 days, 750,
    /// whether or not the void's own note credits its whole invoice.
    /// Rules: R-096
    #[test]
    fn a_void_forgives_only_its_own_document_when_the_charge_is_cut() {
        let owed = expected(None, &[term(3_100, 0, 10), term(4_650, 10, 16)]);
        assert_eq!(owed[0].amount, 1_900);
        let voided = IssuedLine {
            voided: true,
            ..issued("voided", &owed[0].key, 0, 3_100)
        };
        let step = issued("step", &owed[0].key, 10, 1_050);
        let credited = IssuedLine {
            already_credited: 3_100,
            ..voided.clone()
        };
        for voided in [voided, credited] {
            assert_eq!(
                reconcile(&owed, &[voided, step.clone()], TakeBack::Unused(day(16))),
                vec![Adjustment::Credit {
                    line: "step".to_owned(),
                    amount: 750
                }]
            );
        }
    }

    /// 0.04 billed on day 0, raised to 0.10 the same day (0.06 more, voided), ended on day 4: the
    /// charge owes 10 × 4/31 = 1, so 9 comes back as if the void stood: 3 of the live line's
    /// unused time, the voided line's 5 and the 1 left on it forgiven. Once the 3 is credited, a
    /// second pass takes nothing more, where it split the 6 again and took 1 more.
    /// Rules: R-096, R-080
    #[test]
    fn a_take_back_beside_a_void_is_made_once() {
        let owed = expected(None, &[term(10, 0, 4)]);
        assert_eq!(owed[0].amount, 1);
        let first = issued("first", &owed[0].key, 0, 4);
        let raised = IssuedLine {
            voided: true,
            ..issued("raised", &owed[0].key, 0, 6)
        };
        assert_eq!(
            reconcile(
                &owed,
                &[first.clone(), raised.clone()],
                TakeBack::Unused(day(4))
            ),
            vec![Adjustment::Credit {
                line: "first".to_owned(),
                amount: 3
            }]
        );
        let credited = IssuedLine {
            already_credited: 3,
            corrected: 3,
            ..first
        };
        assert!(reconcile(&owed, &[credited, raised], TakeBack::Unused(day(4))).is_empty());
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

    /// The same ending reconciled as each line is issued: the first line's note took its own 15
    /// days, 15000, so the step issued after gives back its own 15000, where its pass split the
    /// rest again over the first line's 16000 left, 9894 and 5106, the first line giving back
    /// days it had used.
    /// Rules: R-096, R-077
    #[test]
    fn a_line_whose_unused_time_a_note_took_gives_nothing_more() {
        let expected = expected(None, &[term(31_000, 0, 10), term(62_000, 10, 16)]);
        let first = IssuedLine {
            already_credited: 15_000,
            corrected: 15_000,
            ..issued("first", &expected[0].key, 0, 31_000)
        };
        let added = issued("added", &expected[0].key, 10, 21_000);
        assert_eq!(
            reconcile(&expected, &[first, added], TakeBack::Unused(day(16))),
            vec![Adjustment::Credit {
                line: "added".to_owned(),
                amount: 15_000
            }]
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
