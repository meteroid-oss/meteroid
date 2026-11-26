//! Coupons: what they take off a subtotal, and how that discount is spread over the lines.

use chrono::NaiveDate;
use common_domain::ids::{AddOnId, CouponId};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use common_utils::decimals::ToSubunit;
use common_utils::integers::ToNonNegativeU64;

/// The taxable amount of each line after `discount` is spread over the lines with a positive
/// subtotal (R-070); `None` where the line keeps the taxable amount it has.
pub fn distribute_discount(subtotals: &[i64], discount: u64) -> Vec<Option<i64>> {
    let mut taxable: Vec<Option<i64>> = vec![None; subtotals.len()];
    if subtotals.is_empty() || discount == 0 {
        return taxable;
    }

    // Calculate total excluding VAT (only positive amounts)
    let total_excl_vat: u64 = subtotals
        .iter()
        .filter(|subtotal| **subtotal > 0)
        .map(|subtotal| *subtotal as u64)
        .sum();

    if total_excl_vat == 0 {
        return taxable;
    }

    // First pass: distribute proportionally
    let mut remaining_discount = discount;

    for (subtotal, taxable) in subtotals.iter().zip(taxable.iter_mut()) {
        if *subtotal <= 0 {
            continue;
        }

        let item_discount = (discount * *subtotal as u64) / total_excl_vat;
        let amount_after_discount = subtotal - item_discount as i64;
        *taxable = Some(amount_after_discount.max(0)); // Ensure non-negative
        remaining_discount = remaining_discount.saturating_sub(item_discount);
    }

    // Second pass: distribute remainder
    if remaining_discount > 0 {
        let mut remainders: Vec<_> = subtotals
            .iter()
            .zip(taxable.iter_mut())
            .filter(|(subtotal, _)| **subtotal > 0)
            .map(|(subtotal, taxable)| {
                let exact_discount = (discount * subtotal.to_non_negative_u64()) % total_excl_vat;
                (taxable, exact_discount)
            })
            .collect();

        remainders.sort_by_key(|(_, remainder)| std::cmp::Reverse(*remainder));

        // Distribute remaining
        for (taxable, _) in remainders.iter_mut().take(remaining_discount as usize) {
            if let Some(amount) = taxable.as_mut() {
                *amount = (*amount - 1).max(0); // Ensure non-negative
            }
        }
    }

    taxable
}

#[derive(Clone, Debug, PartialEq)]
pub enum CouponValue {
    Percentage(Decimal),
    /// A fixed amount, in `currency` at its `exponent`.
    Fixed {
        amount: Decimal,
        currency: String,
        exponent: u8,
    },
    /// Credit granted on redemption: no discount.
    Credit,
}

/// A coupon applicable to the invoice.
#[derive(Clone, Debug, PartialEq)]
pub struct Coupon {
    /// The coupon's own id, which orders coupons of one kind.
    pub coupon_id: CouponId,
    pub value: CouponValue,
    /// The add-ons whose lines alone it discounts; `None`: every line.
    pub add_ons: Option<Vec<AddOnId>>,
    /// A coupon lasting months discounts only the lines whose service period starts before this
    /// day, whatever the invoice's date (R-089); `None`: every period.
    pub until: Option<NaiveDate>,
    /// Discounts only the lines whose service period starts on or after this day: a coupon added
    /// mid-life leaves the periods before it alone; `None`: from any period.
    pub from: Option<NaiveDate>,
}

impl Coupon {
    fn discounts(&self, part: &Part) -> bool {
        self.add_ons
            .as_ref()
            .is_none_or(|ids| part.add_on.is_some_and(|id| ids.contains(&id)))
            && self.covers(part)
    }

    /// Whether `part`'s service period starts within its window.
    fn covers(&self, part: &Part) -> bool {
        part.start.is_none_or(|start| {
            self.from.is_none_or(|from| start >= from)
                && self.until.is_none_or(|until| start < until)
        })
    }
}

/// A line as coupons read it: its subtotal, the add-on it bills, and the day the service period
/// it charges starts (`None`: within any coupon's months).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub subtotal: i64,
    pub add_on: Option<AddOnId>,
    pub start: Option<NaiveDate>,
}

/// What one coupon took off: `coupon` indexes the coupons given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouponDiscount {
    pub coupon: usize,
    pub value: i64,
}

/// The order coupons apply in (R-020): percentages first, then fixed amounts, each kind by its
/// coupon's id so preview and invoice agree (an applied coupon's id is new on each preview).
fn precedence(coupons: &[Coupon]) -> Vec<usize> {
    let mut applicable: Vec<usize> = (0..coupons.len()).collect();
    applicable.sort_by_key(|&i| {
        let type_precedence = match coupons[i].value {
            CouponValue::Percentage(_) => 0u8,
            CouponValue::Fixed { .. } | CouponValue::Credit => 1u8,
        };
        (type_precedence, coupons[i].coupon_id)
    });
    applicable
}

/// What `value` takes off `available`; `None` when it takes nothing off an invoice in
/// `invoice_currency`.
fn off(value: &CouponValue, available: Decimal, invoice_currency: &str) -> Option<Decimal> {
    match value {
        CouponValue::Percentage(percentage) => {
            Some((available * percentage / Decimal::ONE_HUNDRED).min(available))
        }
        CouponValue::Fixed {
            amount,
            currency,
            exponent,
        } => {
            // todo currency conversion
            if currency != invoice_currency {
                return None;
            }
            // R-089: its whole amount on every invoice it lasts; it is no budget.
            let discount_subunits = amount.to_subunit_opt(*exponent).unwrap_or(0);
            Some(Decimal::from(discount_subunits).min(available))
        }
        CouponValue::Credit => None,
    }
}

/// The coupons `coupons` take off `subtotal`, in the order they apply (R-020). A subtotal bills
/// no add-on: a coupon discounting only add-ons takes nothing off it.
pub fn calculate_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[Coupon],
) -> Vec<CouponDiscount> {
    let mut applied = vec![];

    let mut subtotal_subunits = Decimal::from(subtotal);

    for index in precedence(coupons) {
        let applicable_coupon = &coupons[index];
        if subtotal_subunits <= Decimal::ZERO {
            break;
        }
        if applicable_coupon.add_ons.is_some() {
            continue;
        }
        let Some(discount) = off(
            &applicable_coupon.value,
            subtotal_subunits,
            invoice_currency,
        ) else {
            continue;
        };

        subtotal_subunits -= discount;

        applied.push(CouponDiscount {
            coupon: index,
            value: discount.to_i64().unwrap_or(0),
        });
    }

    applied
}

/// What `coupons` take off `parts`, in the order they apply (R-020), and each part's taxable
/// amount after (R-070), `None` where it keeps its subtotal. A coupon discounting only some
/// add-ons, or some periods, is taken off those lines alone, and takes nothing (nor a use) where
/// none is billed.
pub fn discount_parts(
    parts: &[Part],
    invoice_currency: &str,
    coupons: &[Coupon],
) -> (Vec<CouponDiscount>, Vec<Option<i64>>) {
    let subtotals: Vec<i64> = parts.iter().map(|part| part.subtotal).collect();
    if coupons
        .iter()
        .all(|coupon| coupon.add_ons.is_none() && parts.iter().all(|part| coupon.covers(part)))
    {
        let subtotal = subtotals.iter().sum();
        let applied = calculate_coupons_discount(subtotal, invoice_currency, coupons);
        let discount = applied.iter().map(|coupon| coupon.value).sum::<i64>();
        let taxable = distribute_discount(&subtotals, u64::try_from(discount).unwrap_or(0));
        return (applied, taxable);
    }

    let mut applied = vec![];
    let mut left = subtotals.clone();
    let mut touched = vec![false; parts.len()];
    let mut subtotal_subunits = Decimal::from(subtotals.iter().sum::<i64>());
    for index in precedence(coupons) {
        let coupon = &coupons[index];
        if subtotal_subunits <= Decimal::ZERO {
            break;
        }
        let lines: Vec<usize> = (0..parts.len())
            .filter(|i| coupon.discounts(&parts[*i]))
            .collect();
        let available = if coupon.add_ons.is_none() && lines.len() == parts.len() {
            subtotal_subunits
        } else {
            let billed: i64 = lines.iter().map(|i| left[*i].max(0)).sum();
            Decimal::from(billed).min(subtotal_subunits)
        };
        if available <= Decimal::ZERO {
            continue;
        }
        let Some(discount) = off(&coupon.value, available, invoice_currency) else {
            continue;
        };
        subtotal_subunits -= discount;
        let value = discount.to_i64().unwrap_or(0);
        let shares: Vec<i64> = lines.iter().map(|i| left[*i]).collect();
        for (i, taxable) in lines
            .iter()
            .zip(distribute_discount(&shares, value.to_non_negative_u64()))
        {
            if let Some(taxable) = taxable {
                left[*i] = taxable;
                touched[*i] = true;
            }
        }
        applied.push(CouponDiscount {
            coupon: index,
            value,
        });
    }
    let taxable = left
        .into_iter()
        .zip(touched)
        .map(|(left, touched)| touched.then_some(left))
        .collect();
    (applied, taxable)
}

/// The uses left of a subscription's coupons, handed out to its period invoices oldest period
/// first, whatever order they are issued in (R-089): `left[i]` for `coupons[i]`, none when it
/// lasts every invoice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uses {
    left: Vec<Option<u32>>,
}

impl Uses {
    pub fn new(left: Vec<Option<u32>>) -> Self {
        Self { left }
    }

    /// What `coupons[i]` has left.
    pub fn left(&self, i: usize) -> Option<u32> {
        self.left.get(i).copied().flatten()
    }

    /// The coupons the next invoice in period order takes off `parts`, their uses taken: each
    /// with a use left, applied as R-020 applies them.
    pub fn take(&mut self, coupons: &[Coupon], parts: &[Part], currency: &str) -> Vec<usize> {
        self.take_where(coupons, parts, currency, |_| true)
    }

    /// `take` among the coupons `covers` the invoice's period: one lasting months covers none
    /// past them.
    pub fn take_where(
        &mut self,
        coupons: &[Coupon],
        parts: &[Part],
        currency: &str,
        covers: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        let open: Vec<usize> = (0..coupons.len())
            .filter(|i| {
                covers(*i)
                    && self
                        .left
                        .get(*i)
                        .is_some_and(|left| left.is_none_or(|n| n > 0))
            })
            .collect();
        let inputs: Vec<Coupon> = open.iter().map(|i| coupons[*i].clone()).collect();
        let taken: Vec<usize> = discount_parts(parts, currency, &inputs)
            .0
            .iter()
            .map(|applied| open[applied.coupon])
            .collect();
        self.took(&taken);
        taken
    }

    /// Takes a use of each of `taken`: an invoice whose coupons are known.
    pub fn took(&mut self, taken: &[usize]) {
        for i in taken {
            if let Some(Some(n)) = self.left.get_mut(*i) {
                *n = n.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn coupon(id: u128, value: CouponValue) -> Coupon {
        Coupon {
            coupon_id: Uuid::from_u128(id).into(),
            value,
            add_ons: None,
            until: None,
            from: None,
        }
    }

    fn whole(subtotal: i64) -> [Part; 1] {
        [Part {
            subtotal,
            add_on: None,
            start: None,
        }]
    }

    fn usd(amount: Decimal) -> CouponValue {
        CouponValue::Fixed {
            amount,
            currency: "USD".to_string(),
            exponent: 2,
        }
    }

    fn discount(subtotal: i64, coupons: &[Coupon]) -> i64 {
        calculate_coupons_discount(subtotal, "USD", coupons)
            .iter()
            .map(|d| d.value)
            .sum()
    }

    /// A 10 % coupon for two invoices, three drafts open: the two oldest periods take it, the
    /// third none, whichever is issued first; a draft of nothing to discount takes no use.
    /// Rules: R-089
    #[test]
    fn a_coupons_uses_go_to_the_oldest_periods_first() {
        let ten = [coupon(1, CouponValue::Percentage(Decimal::from(10)))];
        let mut uses = Uses::new(vec![Some(2)]);
        assert_eq!(uses.take(&ten, &whole(2_500), "USD"), vec![0]);
        assert_eq!(uses.take(&ten, &whole(0), "USD"), Vec::<usize>::new());
        assert_eq!(uses.take(&ten, &whole(49_597), "USD"), vec![0]);
        assert_eq!(uses.take(&ten, &whole(50_000), "USD"), Vec::<usize>::new());
        assert_eq!(uses.left(0), Some(0));

        let mut forever = Uses::new(vec![None]);
        assert_eq!(forever.take(&ten, &whole(1), "USD"), vec![0]);
        assert_eq!(forever.left(0), None);
    }

    /// A coupon past its months takes nothing off an invoice of a later period, nor a use.
    #[test]
    fn a_coupon_covers_only_the_periods_it_lasts() {
        let ten = [coupon(1, CouponValue::Percentage(Decimal::from(10)))];
        let mut forever = Uses::new(vec![None]);
        assert_eq!(
            forever.take_where(&ten, &whole(1_000), "USD", |_| true),
            vec![0]
        );
        assert_eq!(
            forever.take_where(&ten, &whole(1_000), "USD", |_| false),
            Vec::<usize>::new()
        );
        let mut once = Uses::new(vec![Some(1)]);
        assert!(
            once.take_where(&ten, &whole(1_000), "USD", |_| false)
                .is_empty()
        );
        assert_eq!(once.left(0), Some(1));
    }

    /// A fixed coupon after a 100 % one finds nothing left: the next period takes its use.
    /// Rules: R-089, R-020
    #[test]
    fn a_coupon_that_finds_nothing_left_keeps_its_use() {
        let coupons = [
            coupon(1, CouponValue::Percentage(Decimal::from(100))),
            coupon(2, usd(Decimal::from(5))),
        ];
        let mut uses = Uses::new(vec![Some(1), Some(1)]);
        assert_eq!(uses.take(&coupons, &whole(1_000), "USD"), vec![0]);
        assert_eq!(uses.take(&coupons, &whole(1_000), "USD"), vec![1]);
        assert_eq!(
            uses.take(&coupons, &whole(1_000), "USD"),
            Vec::<usize>::new()
        );
    }

    /// 50% off the Support add-on: 20.00 off its 40.00, nothing off the 100.00 plan or the
    /// 10.00 of usage; an invoice without Support takes nothing, nor a use.
    #[test]
    fn a_coupon_on_an_add_on_discounts_its_lines_alone() {
        let support: AddOnId = Uuid::from_u128(7).into();
        let half = Coupon {
            add_ons: Some(vec![support]),
            ..coupon(1, CouponValue::Percentage(Decimal::from(50)))
        };
        let plan = Part {
            subtotal: 10_000,
            add_on: None,
            start: None,
        };
        let usage = Part {
            subtotal: 1_000,
            add_on: None,
            start: None,
        };
        let line = Part {
            subtotal: 4_000,
            add_on: Some(support),
            start: None,
        };
        let (applied, taxable) =
            discount_parts(&[plan, usage, line], "USD", std::slice::from_ref(&half));
        assert_eq!(
            applied,
            vec![CouponDiscount {
                coupon: 0,
                value: 2_000
            }]
        );
        assert_eq!(taxable, vec![None, None, Some(2_000)]);

        let (applied, taxable) = discount_parts(&[plan, usage], "USD", std::slice::from_ref(&half));
        assert!(applied.is_empty());
        assert_eq!(taxable, vec![None, None]);
        let mut uses = Uses::new(vec![Some(1)]);
        assert_eq!(
            uses.take(std::slice::from_ref(&half), &[plan], "USD"),
            Vec::<usize>::new()
        );
        assert_eq!(uses.left(0), Some(1));
        assert!(calculate_coupons_discount(10_000, "USD", &[half]).is_empty());
    }

    /// 10% off everything, then 5.00 off the add-on, out of the 36.00 its line has left.
    #[test]
    fn a_coupon_on_an_add_on_takes_what_is_left_of_its_lines() {
        let support: AddOnId = Uuid::from_u128(7).into();
        let ten = coupon(1, CouponValue::Percentage(Decimal::from(10)));
        let five = Coupon {
            add_ons: Some(vec![support]),
            ..coupon(2, usd(Decimal::from(5)))
        };
        let parts = [
            Part {
                subtotal: 6_000,
                add_on: None,
                start: None,
            },
            Part {
                subtotal: 4_000,
                add_on: Some(support),
                start: None,
            },
        ];
        let (applied, taxable) = discount_parts(&parts, "USD", &[five, ten]);
        assert_eq!(
            applied,
            vec![
                CouponDiscount {
                    coupon: 1,
                    value: 1_000
                },
                CouponDiscount {
                    coupon: 0,
                    value: 500
                }
            ]
        );
        assert_eq!(taxable, vec![Some(5_400), Some(3_100)]);
    }

    /// "10% off for 12 months" from January 2026, on the invoice of January 2027: 1.00 off
    /// December's 10.00 of usage, nothing off January's fee, whose period starts past its months.
    /// Rules: R-089
    #[test]
    fn a_coupon_lasting_months_discounts_the_lines_of_periods_within_them() {
        let first = |year, month| NaiveDate::from_ymd_opt(year, month, 1);
        let ten = Coupon {
            until: first(2027, 1),
            ..coupon(1, CouponValue::Percentage(Decimal::from(10)))
        };
        let usage = Part {
            subtotal: 1_000,
            add_on: None,
            start: first(2026, 12),
        };
        let fee = Part {
            subtotal: 5_000,
            add_on: None,
            start: first(2027, 1),
        };
        let (applied, taxable) = discount_parts(&[usage, fee], "USD", std::slice::from_ref(&ten));
        assert_eq!(
            applied,
            vec![CouponDiscount {
                coupon: 0,
                value: 100
            }]
        );
        assert_eq!(taxable, vec![Some(900), None]);
        let mut uses = Uses::new(vec![None]);
        assert!(
            uses.take(std::slice::from_ref(&ten), &[fee], "USD")
                .is_empty()
        );
    }

    /// Added mid-life from March: nothing off February's usage billed in arrears, 10% off
    /// March's fee; an invoice of February's usage alone spends none of its uses.
    #[test]
    fn a_coupon_added_mid_life_leaves_the_periods_before_it_alone() {
        let first = |month| NaiveDate::from_ymd_opt(2026, month, 1);
        let ten = Coupon {
            from: first(3),
            ..coupon(1, CouponValue::Percentage(Decimal::from(10)))
        };
        let usage = Part {
            subtotal: 1_000,
            add_on: None,
            start: first(2),
        };
        let fee = Part {
            subtotal: 5_000,
            add_on: None,
            start: first(3),
        };
        let (applied, taxable) = discount_parts(&[usage, fee], "USD", std::slice::from_ref(&ten));
        assert_eq!(
            applied,
            vec![CouponDiscount {
                coupon: 0,
                value: 500
            }]
        );
        assert_eq!(taxable, vec![None, Some(4_500)]);
        let mut uses = Uses::new(vec![Some(1)]);
        assert!(
            uses.take(std::slice::from_ref(&ten), &[usage], "USD")
                .is_empty()
        );
    }

    #[test]
    fn a_coupon_takes_the_last_minor_unit() {
        let full = coupon(1, CouponValue::Percentage(Decimal::from(100)));
        let fixed = coupon(2, usd(Decimal::new(5, 2)));

        assert_eq!(discount(1, &[full]), 1);
        assert_eq!(discount(1, &[fixed]), 1);
    }

    #[test]
    fn two_percentages_commute() {
        let a = coupon(1, CouponValue::Percentage(Decimal::from(10)));
        let b = coupon(2, CouponValue::Percentage(Decimal::from(25)));

        let r1 = discount(10_000, &[a.clone(), b.clone()]);
        let r2 = discount(10_000, &[b, a]);

        assert_eq!(r1, r2);
        // sanity: 25% then 10% off 10000 = 7500 - 750 = 6750 -> discount 3250
        assert_eq!(r1, 3250);
    }

    #[test]
    fn two_fixed_commute() {
        let a = coupon(1, usd(Decimal::from(15)));
        let b = coupon(2, usd(Decimal::from(25)));

        let r1 = discount(10_000, &[a.clone(), b.clone()]);
        let r2 = discount(10_000, &[b, a]);

        assert_eq!(r1, r2);
        // 15 + 25 = 40 USD = 4000 subunits
        assert_eq!(r1, 4000);
    }

    #[test]
    fn two_fixed_commute_even_when_subtotal_floors() {
        // subtotal smaller than total fixed discount -> capped at subtotal regardless of order
        let a = coupon(1, usd(Decimal::from(80)));
        let b = coupon(2, usd(Decimal::from(50)));

        let r1 = discount(10_000, &[a.clone(), b.clone()]);
        let r2 = discount(10_000, &[b, a]);

        assert_eq!(r1, r2);
        assert_eq!(r1, 10_000);
    }

    #[test]
    fn percentage_applied_before_fixed() {
        // Customer-friendly precedence: percentage first.
        // 10% off 10000 = 1000 discount, then 50 USD (5000 subunits) off remaining 9000 = 5000.
        // Total discount = 6000.
        let pct = coupon(2, CouponValue::Percentage(Decimal::from(10)));
        let fix = coupon(1, usd(Decimal::from(50)));

        let r1 = discount(10_000, &[fix.clone(), pct.clone()]);
        let r2 = discount(10_000, &[pct, fix]);

        // Both orderings produce the same result thanks to type precedence.
        assert_eq!(r1, r2);
        assert_eq!(r1, 6000);
    }

    #[test]
    fn a_credit_coupon_discounts_nothing() {
        let credit = coupon(1, CouponValue::Credit);
        assert!(calculate_coupons_discount(10_000, "USD", &[credit]).is_empty());
    }

    #[test]
    fn a_fixed_coupon_in_another_currency_is_skipped() {
        let eur = coupon(
            1,
            CouponValue::Fixed {
                amount: Decimal::from(5),
                currency: "EUR".to_string(),
                exponent: 2,
            },
        );
        assert!(calculate_coupons_discount(10_000, "USD", &[eur]).is_empty());
    }

    fn spread(subtotals: &[i64], discount: u64) -> Vec<i64> {
        distribute_discount(subtotals, discount)
            .into_iter()
            .zip(subtotals)
            .map(|(taxable, subtotal)| taxable.unwrap_or(*subtotal))
            .collect()
    }

    #[test]
    fn test_simple_distribution() {
        // 60€ and 40€, 10€ off so 10%
        assert_eq!(spread(&[6000, 4000], 1000), vec![5400, 3600]);
    }

    #[test]
    fn test_remainder_distribution() {
        let subtotals = [333, 333, 334];
        let taxable = spread(&subtotals, 100);
        assert_eq!(
            subtotals.iter().sum::<i64>(),
            taxable.iter().sum::<i64>() + 100
        );
        assert_eq!(taxable, vec![300, 300, 300]);
    }

    #[test]
    fn test_discount_eq_sub_total() {
        assert_eq!(spread(&[1000, 2000], 3000), vec![0, 0]);
    }

    #[test]
    fn test_discount_gt_sub_total() {
        assert_eq!(spread(&[1000, 2000], 4000), vec![0, 0]);
    }

    #[test]
    fn lines_without_a_positive_subtotal_keep_their_taxable_amount() {
        assert_eq!(
            distribute_discount(&[1000, -500, 0], 100),
            vec![Some(900), None, None]
        );
        assert_eq!(distribute_discount(&[1000], 0), vec![None]);
    }
}
