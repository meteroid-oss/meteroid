//! Coupons: what they take off a subtotal, and how that discount is spread over the lines.

use common_domain::ids::CouponId;
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
}

/// What one coupon took off: `coupon` indexes the coupons given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CouponDiscount {
    pub coupon: usize,
    pub value: i64,
}

/// The coupons `coupons` take off `subtotal`, in the order they apply (R-020).
pub fn calculate_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[Coupon],
) -> Vec<CouponDiscount> {
    // Customer-friendly precedence: percentages first, then fixed amounts.
    // Tiebreak on the underlying coupon.id so preview and invoice paths agree
    // (applied_coupon.id is regenerated per preview and would diverge).
    let mut applicable: Vec<usize> = (0..coupons.len()).collect();
    applicable.sort_by_key(|&i| {
        let type_precedence = match coupons[i].value {
            CouponValue::Percentage(_) => 0u8,
            CouponValue::Fixed { .. } | CouponValue::Credit => 1u8,
        };
        (type_precedence, coupons[i].coupon_id)
    });

    let mut applied = vec![];

    let mut subtotal_subunits = Decimal::from(subtotal);

    for index in applicable {
        let applicable_coupon = &coupons[index];
        if subtotal_subunits <= Decimal::ZERO {
            break;
        }
        let discount = match &applicable_coupon.value {
            CouponValue::Percentage(percentage) => {
                (subtotal_subunits * percentage / Decimal::ONE_HUNDRED).min(subtotal_subunits)
            }
            CouponValue::Fixed {
                amount,
                currency,
                exponent,
            } => {
                // todo currency conversion
                if currency != invoice_currency {
                    continue;
                }

                // R-089: its whole amount on every invoice it lasts; it is no budget.
                let discount_subunits = amount.to_subunit_opt(*exponent).unwrap_or(0);

                Decimal::from(discount_subunits).min(subtotal_subunits)
            }
            CouponValue::Credit => continue,
        };

        subtotal_subunits -= discount;

        applied.push(CouponDiscount {
            coupon: index,
            value: discount.to_i64().unwrap_or(0),
        });
    }

    applied
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

    /// The coupons the next invoice in period order takes off `subtotal`, their uses taken: each
    /// with a use left, applied as R-020 applies them.
    pub fn take(&mut self, coupons: &[Coupon], subtotal: i64, currency: &str) -> Vec<usize> {
        let open: Vec<usize> = (0..coupons.len())
            .filter(|i| {
                self.left
                    .get(*i)
                    .is_some_and(|left| left.is_none_or(|n| n > 0))
            })
            .collect();
        let inputs: Vec<Coupon> = open.iter().map(|i| coupons[*i].clone()).collect();
        let taken: Vec<usize> = calculate_coupons_discount(subtotal, currency, &inputs)
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
        }
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
        assert_eq!(uses.take(&ten, 2_500, "USD"), vec![0]);
        assert_eq!(uses.take(&ten, 0, "USD"), Vec::<usize>::new());
        assert_eq!(uses.take(&ten, 49_597, "USD"), vec![0]);
        assert_eq!(uses.take(&ten, 50_000, "USD"), Vec::<usize>::new());
        assert_eq!(uses.left(0), Some(0));

        let mut forever = Uses::new(vec![None]);
        assert_eq!(forever.take(&ten, 1, "USD"), vec![0]);
        assert_eq!(forever.left(0), None);
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
        assert_eq!(uses.take(&coupons, 1_000, "USD"), vec![0]);
        assert_eq!(uses.take(&coupons, 1_000, "USD"), vec![1]);
        assert_eq!(uses.take(&coupons, 1_000, "USD"), Vec::<usize>::new());
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
