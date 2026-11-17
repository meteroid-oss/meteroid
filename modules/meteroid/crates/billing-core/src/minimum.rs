//! The minimum commitment's true-up of a closed period (R-022, R-023): what its in-scope spend
//! counts, and the shortfall the invoice closing it bills, split over the scoped products (OQ17).

use chrono::NaiveDate;
use common_domain::ids::ProductId;
use rust_decimal::Decimal;

use crate::allocation::largest_remainder;
use crate::proration::{DayFraction, DaySpan};
use crate::rounding::div_round;

/// What a minimum covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    AllComponents,
    Products(Vec<ProductId>),
}

impl Scope {
    fn covers(&self, product: Option<ProductId>) -> bool {
        match self {
            Scope::AllComponents => true,
            Scope::Products(ids) => product.is_some_and(|id| ids.contains(&id)),
        }
    }
}

/// A line of a fee or usage as the minimum reads it, in the figures it was priced in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spend {
    pub product: Option<ProductId>,
    pub from: NaiveDate,
    pub until: NaiveDate,
    /// What it bills before any discount.
    pub subtotal: i64,
    /// What it bills after its coupons, before promotional and prepaid credit.
    pub net: i64,
    /// What the engine's notes and an operator's corrections took back of `subtotal`; never a
    /// goodwill note, nor a void's own (R-082).
    pub corrected: i64,
    /// A fixed fee: a cancellation counts its days served only. Usage counts as billed.
    pub fixed: bool,
    /// Its invoice was voided: no spend (R-096).
    pub voided: bool,
    /// A prorated fixed fee's whole period and price: a cut counts its days served as the note
    /// on it does (R-079).
    pub nominal: Option<Nominal>,
}

/// The whole period a prorated fee's price is for, and that price; for seats bought inside it,
/// the price before them too, the step billing the difference of what is left of each (R-079).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nominal {
    pub period: (NaiveDate, NaiveDate),
    pub full: i64,
    pub base: i64,
}

/// The period a closing invoice trues up, and what its minimum asks of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrueUp {
    pub start: NaiveDate,
    /// Exclusive: the invoice date, the period's end or the day a cancellation cut it short.
    pub end: NaiveDate,
    /// The minimum prorated to the days served ([`floor`]).
    pub floor: i64,
    pub scope: Scope,
    /// The day a cancellation cut the period short: what was billed past it is no spend.
    pub cut: Option<NaiveDate>,
    /// Shortfalls the period's opening invoice prepaid under the rule before the true-up, each
    /// over `[from, until)`.
    pub prepaid: Vec<(NaiveDate, NaiveDate, i64)>,
    /// What earlier true-ups of the period billed, net of their notes: a cancellation cutting it
    /// short bills only what its shortfall exceeds, never crediting one back.
    pub trued_up: i64,
    /// The scoped products the subscription carries, in scope order: a shortfall no product
    /// counted toward is split equally over them.
    pub carried: Vec<ProductId>,
}

/// The minimum over a period: whole for each grid period served in full (`(days, nominal days)`),
/// by days otherwise (R-002).
pub fn floor(minimum: i64, served: &[(i64, i64)]) -> i64 {
    served
        .iter()
        .map(|(days, nominal_days)| {
            if days == nominal_days {
                minimum
            } else {
                DayFraction::new(*days, *nominal_days).prorate(minimum)
            }
        })
        .sum()
}

/// `amount` billed over `[from, until)` less its days past `cut`.
fn served(amount: i64, from: NaiveDate, until: NaiveDate, cut: Option<NaiveDate>) -> i64 {
    match cut {
        Some(cut) if from <= cut && cut < until && amount > 0 => {
            amount - DayFraction::remaining(from, until, cut).prorate(amount)
        }
        _ => amount,
    }
}

impl TrueUp {
    /// What `line` counts toward the floor: its net of coupons less the share of it corrected, a
    /// fixed fee no note corrected only its days served (R-022).
    fn counted(&self, line: &Spend, cut: Option<NaiveDate>) -> Option<(Option<ProductId>, i64)> {
        if line.voided
            || !self.scope.covers(line.product)
            || line.from < self.start
            || line.from >= self.end
        {
            return None;
        }
        let share_of_net = |corrected: i64| {
            if line.subtotal == 0 {
                return 0;
            }
            let share = div_round(
                i128::from(corrected) * i128::from(line.net),
                i128::from(line.subtotal),
            );
            i64::try_from(share).unwrap_or(line.net)
        };
        // A note is what a line bills from then on; a fixed fee no note cut yet counts its days
        // served, each rounded once as its note will.
        let amount = match (line.fixed && line.corrected == 0, line.nominal, cut) {
            (true, Some(nominal), Some(cut))
                if line.from <= cut && cut < line.until && line.net > 0 =>
            {
                let left = DaySpan {
                    period: nominal.period,
                    from: cut,
                    until: line.until,
                };
                let note = left.prorate(nominal.full) - left.prorate(nominal.base);
                line.net - share_of_net(note)
            }
            (true, _, _) => served(line.net, line.from, line.until, cut),
            _ => line.net - share_of_net(line.corrected),
        };
        Some((line.product, amount))
    }

    /// The period's in-scope spend by product: the closing invoice's `document` lines, then what
    /// the subscription's other invoices billed for the period (`prior`).
    pub fn spend(&self, document: &[Spend], prior: &[Spend]) -> Vec<(Option<ProductId>, i64)> {
        document
            .iter()
            .filter_map(|line| self.counted(line, None))
            .chain(prior.iter().filter_map(|line| self.counted(line, self.cut)))
            .collect()
    }

    /// What the opening invoice prepaid of the floor, for the days served.
    pub fn already_billed(&self) -> i64 {
        self.prepaid
            .iter()
            .map(|(from, until, amount)| served(*amount, *from, *until, self.cut))
            .sum()
    }

    /// The shortfall of `counted` spend: what the floor asks above it and above what was prepaid
    /// or trued up already. Never negative: a true-up is never credited back.
    pub fn shortfall(&self, counted: i64) -> i64 {
        (self.floor - counted - self.already_billed() - self.trued_up).max(0)
    }

    /// The lines the closing invoice bills: none when the spend meets the floor, else the
    /// shortfall, outside the coupons' base (R-023), by product (OQ17).
    pub fn lines(&self, document: &[Spend], prior: &[Spend]) -> Vec<(Option<ProductId>, i64)> {
        let spend = self.spend(document, prior);
        let shortfall = self.shortfall(spend.iter().map(|(_, amount)| amount).sum());
        if shortfall == 0 {
            return Vec::new();
        }
        let weights: Vec<(ProductId, i64)> = spend
            .into_iter()
            .filter_map(|(product, amount)| product.map(|product| (product, amount)))
            .collect();
        split(&self.scope, &weights, &self.carried, shortfall)
    }
}

/// How a shortfall is billed: one product-less line for an all-components minimum; for a
/// product-scoped one, a line per scoped product pro rata to what it counted toward the floor
/// (R-003), equally over the scoped products when none counted anything.
pub fn split(
    scope: &Scope,
    weights: &[(ProductId, i64)],
    carried: &[ProductId],
    shortfall: i64,
) -> Vec<(Option<ProductId>, i64)> {
    let Scope::Products(ids) = scope else {
        return vec![(None, shortfall)];
    };
    let mut by_product: Vec<(ProductId, i64)> = Vec::new();
    for (product, amount) in weights.iter().filter(|(p, _)| ids.contains(p)) {
        match by_product.iter_mut().find(|(p, _)| p == product) {
            Some(entry) => entry.1 += amount,
            None => by_product.push((*product, *amount)),
        }
    }
    by_product.retain(|(_, amount)| *amount > 0);
    if by_product.is_empty() {
        let pool: Vec<ProductId> = if carried.is_empty() {
            ids.clone()
        } else {
            carried.to_vec()
        };
        by_product = pool.into_iter().map(|product| (product, 1)).collect();
    }
    if by_product.is_empty() {
        return vec![(None, shortfall)];
    }
    let total: i64 = by_product.iter().map(|(_, weight)| weight).sum();
    let exact: Vec<Decimal> = by_product
        .iter()
        .map(|(_, weight)| Decimal::from(shortfall) * Decimal::from(*weight) / Decimal::from(total))
        .collect();
    by_product
        .iter()
        .zip(largest_remainder(&exact))
        .filter(|(_, amount)| *amount != 0)
        .map(|((product, _), amount)| (Some(*product), amount))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, m, day).unwrap()
    }

    fn january(floor: i64) -> TrueUp {
        TrueUp {
            start: d(1, 1),
            end: d(2, 1),
            floor,
            scope: Scope::AllComponents,
            cut: None,
            prepaid: Vec::new(),
            trued_up: 0,
            carried: Vec::new(),
        }
    }

    fn line(from: NaiveDate, until: NaiveDate, subtotal: i64, fixed: bool) -> Spend {
        Spend {
            product: None,
            from,
            until,
            subtotal,
            net: subtotal,
            corrected: 0,
            fixed,
            voided: false,
            nominal: None,
        }
    }

    /// A 10 % coupon on January's 10.00 fee and 50.00 of usage under a 100.00 minimum: spend
    /// counts 9.00 + 45.00, so the shortfall is 46.00, its own line beside February's fee.
    /// Rules: R-022, R-023
    #[test]
    fn spend_is_counted_net_of_coupons() {
        let fee = Spend {
            net: 900,
            ..line(d(1, 1), d(2, 1), 1_000, true)
        };
        let usage = Spend {
            net: 4_500,
            ..line(d(1, 1), d(2, 1), 5_000, false)
        };
        let february = Spend {
            net: 900,
            ..line(d(2, 1), d(3, 1), 1_000, true)
        };
        assert_eq!(
            january(10_000).lines(&[usage, february], &[fee]),
            vec![(None, 4_600)]
        );
    }

    /// A 60.00 fee of which the engine gave back 29.03 at a change, beside 14.52 the new fee
    /// billed, under a 100.00 minimum: the fee counts 30.97, so the shortfall is 54.51.
    /// Rules: R-022
    #[test]
    fn spend_is_counted_net_of_corrections() {
        let old = Spend {
            corrected: 2_903,
            ..line(d(1, 1), d(2, 1), 6_000, true)
        };
        let new = line(d(1, 17), d(2, 1), 1_452, true);
        assert_eq!(january(10_000).lines(&[], &[old, new]), vec![(None, 5_451)]);
    }

    /// A correction of a discounted line takes the same share of its net: 10.00 of a 40.00 line
    /// billed 30.00 after its coupon leaves 22.50.
    /// Rules: R-022
    #[test]
    fn a_correction_of_a_discounted_line_takes_its_share_of_the_net() {
        let corrected = Spend {
            net: 3_000,
            corrected: 1_000,
            ..line(d(1, 1), d(2, 1), 4_000, true)
        };
        assert_eq!(
            january(10_000).spend(&[], &[corrected]),
            vec![(None, 2_250)]
        );
    }

    /// A voided 60.00 invoice is no spend: the 100.00 minimum is trued up whole.
    /// Rules: R-022, R-096
    #[test]
    fn a_voided_line_is_no_spend() {
        let voided = Spend {
            voided: true,
            ..line(d(1, 1), d(2, 1), 6_000, true)
        };
        assert_eq!(january(10_000).lines(&[], &[voided]), vec![(None, 10_000)]);
    }

    /// Cancelled on Jan 10, a 29.00 fee counts its 9 days served, 842, whether or not the
    /// cancellation's note took the rest back yet; usage counts as billed.
    /// Rules: R-022
    #[test]
    fn a_cancellation_counts_a_fixed_fee_by_its_days_served() {
        let cut = TrueUp {
            end: d(1, 10),
            cut: Some(d(1, 10)),
            ..january(29_032)
        };
        let fee = line(d(1, 1), d(2, 1), 2_900, true);
        let noted = Spend {
            corrected: 2_058,
            ..fee.clone()
        };
        let usage = line(d(1, 1), d(1, 10), 9_000, false);
        for fee in [fee, noted] {
            assert_eq!(
                cut.spend(&[], &[fee, usage.clone()]),
                vec![(None, 842), (None, 9_000)]
            );
        }
    }

    /// A 120.00 yearly fee added on Jan 16, 2024 bills its stub to Jan 1, 2025, 12000 × 351/366 =
    /// 11508. Cancelled at January's end, it counts what the note on it leaves, 11508 − 12000 ×
    /// 335/366 = 11508 − 10984 = 524, before that note is written as after.
    /// Rules: R-022, R-079
    #[test]
    fn a_cut_stub_counts_its_days_served_over_its_whole_period_as_its_note_does() {
        let cut = TrueUp {
            cut: Some(d(2, 1)),
            ..january(10_000)
        };
        let next_year = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let stub = Spend {
            nominal: Some(Nominal {
                period: (d(1, 1), next_year),
                full: 12_000,
                base: 0,
            }),
            ..line(d(1, 16), next_year, 11_508, true)
        };
        let noted = Spend {
            corrected: 10_984,
            ..stub.clone()
        };
        for stub in [stub, noted] {
            assert_eq!(cut.spend(&[], &[stub]), vec![(None, 524)]);
        }
    }

    /// One 10.00 seat from Jan 5, 2025, three more bought on Jan 10 for 4000 × 26/31 − 1000 ×
    /// 26/31 = 3355 − 839 = 2516, cancelled from Jan 16 before any note: the seat counts
    /// 1000 − 645 = 355, the step 2516 − (2581 − 645) = 580 as its note will leave it, where its
    /// days served read 581.
    /// Rules: R-022, R-079
    #[test]
    fn a_seat_step_counts_what_its_note_will_leave() {
        let day = |m, d| NaiveDate::from_ymd_opt(2025, m, d).unwrap();
        let period = (day(1, 5), day(2, 5));
        let cut = TrueUp {
            start: day(1, 5),
            end: day(1, 16),
            cut: Some(day(1, 16)),
            ..january(0)
        };
        let seat = Spend {
            nominal: Some(Nominal {
                period,
                full: 1_000,
                base: 0,
            }),
            ..line(day(1, 5), day(2, 5), 1_000, true)
        };
        let step = Spend {
            nominal: Some(Nominal {
                period,
                full: 4_000,
                base: 1_000,
            }),
            ..line(day(1, 10), day(2, 5), 2_516, true)
        };
        assert_eq!(
            cut.spend(&[], &[seat, step]),
            vec![(None, 355), (None, 580)]
        );
    }

    /// Cancelled on Aug 28 after 6 days of a period of 31 from Aug 22: the floor is rounded once,
    /// 100000 × 6/31 = 19354.84, so 19355. A fee its note left 2205 counts 2205, what it bills,
    /// even where its days served would read 2204: the shortfall is 19355 − 2205 = 17150.
    /// Rules: R-022, OQ14
    #[test]
    fn a_fee_its_note_cut_counts_what_it_bills() {
        let period = TrueUp {
            start: d(8, 22),
            end: d(8, 28),
            floor: floor(100_000, &[(6, 31)]),
            cut: Some(d(8, 28)),
            ..january(0)
        };
        assert_eq!(period.floor, 19_355);
        let fee = Spend {
            corrected: 11_389 - 2_205,
            ..line(d(8, 22), d(9, 22), 11_389, true)
        };
        assert_eq!(period.lines(&[], &[fee]), vec![(None, 17_150)]);
    }

    /// A shortfall the opening invoice prepaid nets the true-up, for the days served.
    /// Rules: R-022
    #[test]
    fn a_prepaid_shortfall_nets_the_true_up() {
        let prepaid = TrueUp {
            prepaid: vec![(d(1, 1), d(2, 1), 1_500)],
            ..january(5_000)
        };
        let fee = line(d(1, 1), d(2, 1), 3_500, true);
        assert!(prepaid.lines(&[], &[fee]).is_empty());
    }

    /// Trued up 30.01 when it closed, a period a backdated cancellation cut to 16 of its 31 days
    /// owes 5000 × 16/31 = 2581 less the 10.32 its notes left: 15.49, under what was trued up, so
    /// nothing more. Had it trued up 10.00, only the 5.49 above it is billed.
    /// Rules: R-022
    #[test]
    fn a_cut_period_bills_only_what_its_shortfall_exceeds_its_true_up() {
        let cut = TrueUp {
            start: d(1, 28),
            end: d(2, 13),
            floor: floor(5_000, &[(16, 31)]),
            cut: Some(d(2, 13)),
            trued_up: 3_001,
            ..january(0)
        };
        assert_eq!(cut.floor, 2_581);
        let fee = Spend {
            corrected: 1_999 - 1_032,
            ..line(d(1, 28), d(2, 28), 1_999, true)
        };
        assert!(cut.lines(&[], std::slice::from_ref(&fee)).is_empty());
        let less = TrueUp {
            trued_up: 1_000,
            ..cut
        };
        assert_eq!(less.lines(&[], &[fee]), vec![(None, 549)]);
    }

    /// A product-scoped 100.00 minimum after 29.00 and 10.00 of its products' spend: 61.00
    /// split 45.36 and 15.64, as each counted.
    /// Rules: OQ17, R-003
    #[test]
    fn a_product_scoped_shortfall_is_split_by_what_each_product_counted() {
        let (fee, seats): (ProductId, ProductId) =
            (Uuid::from_u128(1).into(), Uuid::from_u128(2).into());
        let scoped = TrueUp {
            scope: Scope::Products(vec![fee, seats]),
            ..january(10_000)
        };
        let of = |product, amount| Spend {
            product: Some(product),
            ..line(d(1, 1), d(2, 1), amount, true)
        };
        assert_eq!(
            scoped.lines(&[], &[of(fee, 2_900), of(seats, 1_000)]),
            vec![(Some(fee), 4_536), (Some(seats), 1_564)]
        );
    }

    /// Rules: R-022
    #[test]
    fn the_floor_is_whole_for_a_full_period_and_by_days_otherwise() {
        assert_eq!(floor(100_000, &[(31, 31)]), 100_000);
        assert_eq!(floor(100_000, &[(9, 31)]), 29_032);
    }
}
