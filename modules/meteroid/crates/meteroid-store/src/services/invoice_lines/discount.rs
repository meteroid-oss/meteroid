use crate::domain::coupons::{AppliedCouponsDiscount, CouponDiscount};
use crate::domain::{AppliedCouponDetailed, CouponLineItem, LineItem};
use common_domain::ids::BaseId;
use common_utils::decimals::ToSubunit;
use common_utils::integers::ToNonNegativeU64;
use itertools::Itertools;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

pub fn distribute_discount(line_items: Vec<LineItem>, discount: u64) -> Vec<LineItem> {
    if line_items.is_empty() || discount == 0 {
        return line_items;
    }

    // Calculate total excluding VAT (only positive amounts)
    let total_excl_vat: u64 = line_items
        .iter()
        .filter(|item| item.amount_subtotal > 0)
        .map(|item| item.amount_subtotal as u64)
        .sum();

    if total_excl_vat == 0 {
        return line_items;
    }

    // First pass: distribute proportionally
    let mut remaining_discount = discount;

    let mut line_items = line_items.clone();

    for item in &mut line_items {
        if item.amount_subtotal <= 0 {
            continue;
        }

        let item_discount = (discount * item.amount_subtotal as u64) / total_excl_vat;
        let amount_after_discount = item.amount_subtotal - item_discount as i64;
        item.taxable_amount = amount_after_discount.max(0); // Ensure non-negative
        remaining_discount = remaining_discount.saturating_sub(item_discount);
    }

    // Second pass: distribute remainder
    if remaining_discount > 0 {
        let mut remainders: Vec<_> = line_items
            .iter_mut()
            .filter(|item| item.amount_subtotal > 0)
            .map(|item| {
                let exact_discount =
                    (discount * item.amount_subtotal.to_non_negative_u64()) % total_excl_vat;
                (item, exact_discount)
            })
            .collect();

        remainders.sort_by_key(|(_, remainder)| std::cmp::Reverse(*remainder));

        // Distribute remaining
        for (item, _) in remainders.iter_mut().take(remaining_discount as usize) {
            item.taxable_amount = (item.taxable_amount - 1).max(0); // Ensure non-negative
        }
    }

    line_items
}

pub fn calculate_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[AppliedCouponDetailed],
) -> AppliedCouponsDiscount {
    // Customer-friendly precedence: percentages first, then fixed amounts.
    // Tiebreak on the underlying coupon.id so preview and invoice paths agree
    // (applied_coupon.id is regenerated per preview and would diverge).
    let applicable_coupons: Vec<&AppliedCouponDetailed> = coupons
        .iter()
        .filter(|x| x.is_invoice_applicable())
        .sorted_by_key(|x| {
            let type_precedence = match x.coupon.discount {
                CouponDiscount::Percentage(_) => 0u8,
                CouponDiscount::Fixed { .. } => 1u8,
            };
            (type_precedence, x.coupon.id.as_uuid())
        })
        .collect::<Vec<_>>();

    let mut applied_coupons_items = vec![];

    let mut subtotal_subunits = Decimal::from(subtotal);

    for applicable_coupon in applicable_coupons {
        if subtotal_subunits <= Decimal::ONE {
            break;
        }
        let discount = match &applicable_coupon.coupon.discount {
            CouponDiscount::Percentage(percentage) => {
                (subtotal_subunits * percentage / Decimal::ONE_HUNDRED).min(subtotal_subunits)
            }
            CouponDiscount::Fixed { amount, currency } => {
                // todo currency conversion
                if currency != invoice_currency {
                    continue;
                }
                // todo domain should use Currency type instead of string
                let cur = rusty_money::iso::find(currency).unwrap_or(rusty_money::iso::USD);

                let consumed_amount = &applicable_coupon
                    .applied_coupon
                    .applied_amount
                    .unwrap_or(Decimal::ZERO);

                let discount_subunits = (amount - consumed_amount)
                    .to_subunit_opt(cur.exponent as u8)
                    .unwrap_or(0);

                Decimal::from(discount_subunits).min(subtotal_subunits)
            }
        };

        subtotal_subunits -= discount;

        let discount = discount.to_i64().unwrap_or(0);
        applied_coupons_items.push(CouponLineItem {
            coupon_id: applicable_coupon.coupon.id,
            applied_coupon_id: applicable_coupon.applied_coupon.id,
            name: format!("Coupon ({})", applicable_coupon.coupon.code), // TODO allow defining a name in coupon
            code: applicable_coupon.coupon.code.clone(),
            value: discount,
            discount: applicable_coupon.coupon.discount.clone(),
        });
    }

    AppliedCouponsDiscount {
        discount_subunit: applied_coupons_items.iter().map(|x| x.value).sum(),
        applied_coupons: applied_coupons_items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coupons::Coupon;
    use crate::domain::subscription_coupons::AppliedCoupon;
    use common_domain::ids::{AppliedCouponId, CouponId, CustomerId, SubscriptionId, TenantId};
    use rust_decimal::Decimal;

    fn applied_coupon(code: &str, discount: CouponDiscount) -> AppliedCouponDetailed {
        let now = chrono::Utc::now().naive_utc();
        let coupon_id = CouponId::new();
        AppliedCouponDetailed {
            coupon: Coupon {
                id: coupon_id,
                code: code.to_string(),
                description: String::new(),
                tenant_id: TenantId::new(),
                discount,
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: true,
                disabled: false,
                created_at: now,
                updated_at: now,
                last_redemption_at: None,
                archived_at: None,
                redemption_count: 0,
                plan_ids: vec![],
            },
            applied_coupon: AppliedCoupon {
                id: AppliedCouponId::new(),
                coupon_id,
                customer_id: CustomerId::new(),
                subscription_id: SubscriptionId::new(),
                is_active: true,
                applied_amount: None,
                applied_count: Some(0),
                last_applied_at: None,
                created_at: now,
            },
        }
    }

    #[test]
    fn two_percentages_commute() {
        let a = applied_coupon("PCT_A", CouponDiscount::Percentage(Decimal::from(10)));
        let b = applied_coupon("PCT_B", CouponDiscount::Percentage(Decimal::from(25)));

        let r1 = calculate_coupons_discount(10_000, "USD", &[a.clone(), b.clone()]);
        let r2 = calculate_coupons_discount(10_000, "USD", &[b, a]);

        assert_eq!(r1.discount_subunit, r2.discount_subunit);
        // sanity: 25% then 10% off 10000 = 7500 - 750 = 6750 -> discount 3250
        assert_eq!(r1.discount_subunit, 3250);
    }

    #[test]
    fn two_fixed_commute() {
        let a = applied_coupon(
            "FIX_A",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(15),
            },
        );
        let b = applied_coupon(
            "FIX_B",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(25),
            },
        );

        let r1 = calculate_coupons_discount(10_000, "USD", &[a.clone(), b.clone()]);
        let r2 = calculate_coupons_discount(10_000, "USD", &[b, a]);

        assert_eq!(r1.discount_subunit, r2.discount_subunit);
        // 15 + 25 = 40 USD = 4000 subunits
        assert_eq!(r1.discount_subunit, 4000);
    }

    #[test]
    fn two_fixed_commute_even_when_subtotal_floors() {
        // subtotal smaller than total fixed discount -> capped at subtotal regardless of order
        let a = applied_coupon(
            "FIX_A",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(80),
            },
        );
        let b = applied_coupon(
            "FIX_B",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(50),
            },
        );

        let r1 = calculate_coupons_discount(10_000, "USD", &[a.clone(), b.clone()]);
        let r2 = calculate_coupons_discount(10_000, "USD", &[b, a]);

        assert_eq!(r1.discount_subunit, r2.discount_subunit);
        assert_eq!(r1.discount_subunit, 10_000);
    }

    #[test]
    fn percentage_applied_before_fixed() {
        // Customer-friendly precedence: percentage first.
        // 10% off 10000 = 1000 discount, then 50 USD (5000 subunits) off remaining 9000 = 5000.
        // Total discount = 6000.
        let pct = applied_coupon("PCT", CouponDiscount::Percentage(Decimal::from(10)));
        let fix = applied_coupon(
            "FIX",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(50),
            },
        );

        let r1 = calculate_coupons_discount(10_000, "USD", &[fix.clone(), pct.clone()]);
        let r2 = calculate_coupons_discount(10_000, "USD", &[pct, fix]);

        // Both orderings produce the same result thanks to type precedence.
        assert_eq!(r1.discount_subunit, r2.discount_subunit);
        assert_eq!(r1.discount_subunit, 6000);
    }

    fn new_line_item(amount_subtotal: i64) -> LineItem {
        LineItem {
            local_id: "test".to_string(),
            name: "test".to_string(),
            amount_subtotal,
            taxable_amount: amount_subtotal,
            tax_amount: 0,
            tax_rate: Decimal::ZERO,
            quantity: None,
            unit_price: None,
            start_date: chrono::NaiveDate::MIN,
            end_date: chrono::NaiveDate::MIN,
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            amount_total: amount_subtotal,
            group_by_dimensions: None,
            tax_details: vec![],
        }
    }

    #[test]
    fn test_simple_distribution() {
        let items = vec![
            new_line_item(6000), // 60€
            new_line_item(4000), // 40€
        ];

        let items = distribute_discount(items, 1000); // 10€ so 10%

        assert_eq!(items[0].taxable_amount, 5400); // 60€ - 6€
        assert_eq!(items[1].taxable_amount, 3600); // 40€ - 4€
    }

    #[test]
    fn test_remainder_distribution() {
        let items = vec![
            new_line_item(333), // 3.33€
            new_line_item(333), // 3.33€
            new_line_item(334), // 3.34€
        ];

        let sum_before = items.iter().map(|item| item.amount_subtotal).sum::<i64>();

        let discount_amount = 100; // 1€  so 10%

        let items = distribute_discount(items, discount_amount);

        let sum_after = items.iter().map(|item| item.taxable_amount).sum::<i64>();

        assert_eq!(sum_before, sum_after + discount_amount as i64);

        assert_eq!(items[0].taxable_amount, 300); // 3.33€ - 0.33€
        assert_eq!(items[1].taxable_amount, 300); // 3.33€ - 0.33€
        assert_eq!(items[2].taxable_amount, 300); // 3.34€ - 0.34€
    }

    #[test]
    fn test_discount_eq_sub_total() {
        let items = vec![
            new_line_item(1000), // 10.00€
            new_line_item(2000), // 20.00€
        ];

        let discount_amount = 3000; // 30.00€

        let items = distribute_discount(items, discount_amount);

        let sum_after = items.iter().map(|item| item.taxable_amount).sum::<i64>();

        assert_eq!(sum_after, 0);
        assert_eq!(items[0].taxable_amount, 0);
        assert_eq!(items[1].taxable_amount, 0);
    }

    #[test]
    fn test_discount_gt_sub_total() {
        let items = vec![
            new_line_item(1000), // 10.00€
            new_line_item(2000), // 20.00€
        ];

        let discount_amount = 4000; // 40.00€

        let items = distribute_discount(items, discount_amount);

        let sum_after = items.iter().map(|item| item.taxable_amount).sum::<i64>();

        assert_eq!(sum_after, 0);
        assert_eq!(items[0].taxable_amount, 0);
        assert_eq!(items[1].taxable_amount, 0);
    }
}
