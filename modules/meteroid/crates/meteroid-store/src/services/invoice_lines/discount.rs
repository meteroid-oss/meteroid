use crate::domain::coupons::{AppliedCouponsDiscount, CouponDiscount};
use crate::domain::{AppliedCouponDetailed, CouponLineItem, LineItem};
use billing_core::assembly::discount as core;

pub fn distribute_discount(line_items: Vec<LineItem>, discount: u64) -> Vec<LineItem> {
    let subtotals: Vec<i64> = line_items.iter().map(|item| item.amount_subtotal).collect();
    let taxable = core::distribute_discount(&subtotals, discount);
    line_items
        .into_iter()
        .zip(taxable)
        .map(|(mut item, taxable)| {
            if let Some(taxable) = taxable {
                item.taxable_amount = taxable;
            }
            item
        })
        .collect()
}

/// A coupon as `billing_core` reads it; a fixed amount in its currency's exponent (KW-02).
fn to_core(coupon: &AppliedCouponDetailed) -> core::Coupon {
    core::Coupon {
        coupon_id: coupon.coupon.id,
        value: match &coupon.coupon.discount {
            CouponDiscount::Percentage(percentage) => core::CouponValue::Percentage(*percentage),
            CouponDiscount::Fixed { amount, currency } => core::CouponValue::Fixed {
                amount: *amount,
                currency: currency.clone(),
                // todo domain should use Currency type instead of string
                exponent: meteroid_money::iso::find(currency)
                    .unwrap_or(meteroid_money::iso::USD)
                    .exponent as u8,
            },
            CouponDiscount::Credit { .. } => core::CouponValue::Credit,
        },
    }
}

/// The coupons that apply to the invoice being computed (R-089).
pub fn applicable_coupons(coupons: &[AppliedCouponDetailed]) -> Vec<&AppliedCouponDetailed> {
    coupons
        .iter()
        .filter(|x| x.is_invoice_applicable())
        .collect()
}

/// `applicable` as `billing_core` reads them.
pub fn coupon_inputs(applicable: &[&AppliedCouponDetailed]) -> Vec<core::Coupon> {
    applicable.iter().map(|c| to_core(c)).collect()
}

/// The coupon rows of what `applied` took off, `applied` indexing `applicable`.
pub fn coupon_rows(
    applicable: &[&AppliedCouponDetailed],
    applied: &[core::CouponDiscount],
) -> Vec<CouponLineItem> {
    applied
        .iter()
        .map(|applied| {
            let applicable_coupon = applicable[applied.coupon];
            CouponLineItem {
                coupon_id: applicable_coupon.coupon.id,
                applied_coupon_id: applicable_coupon.applied_coupon.id,
                name: format!("Coupon ({})", applicable_coupon.coupon.code), // TODO allow defining a name in coupon
                code: applicable_coupon.coupon.code.clone(),
                value: applied.value,
                quoted_value: None,
                discount: applicable_coupon.coupon.discount.clone(),
            }
        })
        .collect()
}

pub fn calculate_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[AppliedCouponDetailed],
) -> AppliedCouponsDiscount {
    let applicable_coupons = applicable_coupons(coupons);
    let inputs = coupon_inputs(&applicable_coupons);
    let applied_coupons_items = coupon_rows(
        &applicable_coupons,
        &core::calculate_coupons_discount(subtotal, invoice_currency, &inputs),
    );

    AppliedCouponsDiscount {
        discount_subunit: applied_coupons_items.iter().map(|x| x.value).sum(),
        applied_coupons: applied_coupons_items,
    }
}

/// What `coupons` will take off `subtotal` on the invoice that follows one which consumed
/// `consumed`. Counts the application finalization records (see `refresh_applied_coupons`) so a
/// coupon that has reached its cycle limit is accounted for.
pub fn calculate_next_invoice_coupons_discount(
    subtotal: i64,
    invoice_currency: &str,
    coupons: &[AppliedCouponDetailed],
    consumed: &[CouponLineItem],
) -> i64 {
    let next: Vec<AppliedCouponDetailed> = coupons
        .iter()
        .cloned()
        .map(|mut c| {
            if consumed
                .iter()
                .any(|l| l.applied_coupon_id == c.applied_coupon.id)
            {
                let applied = &mut c.applied_coupon;
                applied.applied_count = Some(applied.applied_count.unwrap_or(0) + 1);
            }
            c
        })
        .collect();

    calculate_coupons_discount(subtotal, invoice_currency, &next)
        .discount_subunit
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coupons::Coupon;
    use crate::domain::subscription_coupons::AppliedCoupon;
    use common_domain::ids::BaseId;
    use common_domain::ids::{AppliedCouponId, CouponId, CustomerId, SubscriptionId, TenantId};
    use rust_decimal::Decimal;

    fn applied_coupon(code: &str, discount: CouponDiscount) -> AppliedCouponDetailed {
        let now = common_clock::now().naive_utc();
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
    fn a_coupon_takes_the_last_minor_unit() {
        let full = applied_coupon("FULL", CouponDiscount::Percentage(Decimal::from(100)));
        let fixed = applied_coupon(
            "FIXED",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::new(5, 2),
            },
        );

        assert_eq!(
            calculate_coupons_discount(1, "USD", &[full]).discount_subunit,
            1
        );
        assert_eq!(
            calculate_coupons_discount(1, "USD", &[fixed]).discount_subunit,
            1
        );
    }

    #[test]
    fn a_credit_coupon_discounts_nothing() {
        let credit = applied_coupon(
            "WELCOME",
            CouponDiscount::Credit {
                currency: "USD".to_string(),
                amount: Decimal::from(25),
                expires_in_days: None,
                scope: Default::default(),
            },
        );
        let result = calculate_coupons_discount(10_000, "USD", &[credit]);
        assert_eq!(result.discount_subunit, 0);
        assert!(result.applied_coupons.is_empty());
    }

    #[test]
    fn next_invoice_discount_ignores_coupons_unused_today() {
        // Nothing billed today (usage in arrears): the coupon is untouched and fully applies to
        // the next invoice's 50.00 floor.
        let pct = applied_coupon("PCT", CouponDiscount::Percentage(Decimal::from(10)));
        assert_eq!(
            calculate_next_invoice_coupons_discount(5_000, "USD", &[pct], &[]),
            500
        );
    }

    #[test]
    fn next_invoice_discount_drops_a_coupon_spent_today() {
        // A one-cycle coupon consumed on today's invoice no longer applies next time.
        let mut once = applied_coupon("ONCE", CouponDiscount::Percentage(Decimal::from(10)));
        once.coupon.recurring_value = Some(1);
        let today = calculate_coupons_discount(2_000, "USD", std::slice::from_ref(&once));
        assert_eq!(today.discount_subunit, 200);

        assert_eq!(
            calculate_next_invoice_coupons_discount(5_000, "USD", &[once], &today.applied_coupons),
            0
        );
    }

    fn new_line_item(amount_subtotal: i64) -> LineItem {
        LineItem {
            local_id: "test".to_string(),
            name: "test".to_string(),
            charge_type: None,
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
            kind: crate::domain::LineItemKind::Regular,
            credit_grant_amount: None,
            credit_grant_expiry: None,
            credit_grant_included: false,
            taxed_at_purchase: false,
            tax_inclusive_quote: None,
            credit_unit: None,
            credit_grant_unit: None,
            credit_grant_rollover: None,
            promotional_credit: None,
            subscription_id: None,
            usage_ends_at: None,
            usage_breakdown: None,
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
    fn a_fixed_coupon_gives_its_whole_amount_on_each_invoice_it_lasts() {
        let mut fixed = applied_coupon(
            "TWICE",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(10),
            },
        );
        fixed.coupon.recurring_value = Some(2);

        let mut taken = vec![];
        for _ in 0..3 {
            let invoice = calculate_coupons_discount(3_000, "USD", std::slice::from_ref(&fixed));
            taken.push(invoice.discount_subunit);
            // What finalization records: one more invoice, and the amount given.
            if invoice.discount_subunit > 0 {
                let applied = &mut fixed.applied_coupon;
                applied.applied_count = Some(applied.applied_count.unwrap_or(0) + 1);
                applied.applied_amount = Some(
                    applied.applied_amount.unwrap_or_default()
                        + Decimal::new(invoice.discount_subunit, 2),
                );
            }
        }
        assert_eq!(taken, vec![1_000, 1_000, 0]);
    }

    #[test]
    fn next_invoice_discount_keeps_a_fixed_coupon_whole() {
        let mut fixed = applied_coupon(
            "TWICE",
            CouponDiscount::Fixed {
                currency: "USD".to_string(),
                amount: Decimal::from(10),
            },
        );
        fixed.coupon.recurring_value = Some(2);
        let today = calculate_coupons_discount(3_000, "USD", std::slice::from_ref(&fixed));
        assert_eq!(today.discount_subunit, 1_000);

        assert_eq!(
            calculate_next_invoice_coupons_discount(
                3_000,
                "USD",
                std::slice::from_ref(&fixed),
                &today.applied_coupons
            ),
            1_000
        );
        fixed.applied_coupon.applied_count = Some(1);
        assert_eq!(
            calculate_next_invoice_coupons_discount(3_000, "USD", &[fixed], &today.applied_coupons),
            0
        );
    }
}
