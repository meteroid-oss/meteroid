//! Coupons: what they take off a subtotal, and how that discount is spread over the lines.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use uuid::Uuid;

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
    pub coupon_id: Uuid,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn coupon(id: u128, value: CouponValue) -> Coupon {
        Coupon {
            coupon_id: Uuid::from_u128(id),
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
