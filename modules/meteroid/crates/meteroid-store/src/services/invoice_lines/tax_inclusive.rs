//! The store's lines netted by `billing_core::assembly::tax_inclusive`.

use crate::domain::{CouponLineItem, LineItem, SubLineAttributes, TaxInclusiveQuote};
use billing_core::assembly::tax_inclusive as core;

/// Rewrites a line in net terms once the engine has carved the tax out of it.
///
/// `quoted_taxable` is the line's taxable amount as priced (tax included, after discount);
/// `line.taxable_amount` is already the net the engine reported. Returns how much of the
/// line's discount was tax — the amount the invoice-level discount shrinks by.
///
/// A line that bore no tax is left exactly as priced: a reverse-charge, exempt or export
/// customer pays the quoted price in full, and there is nothing to net.
pub(crate) fn net_line(line: &mut LineItem, quoted_taxable: i64) -> i64 {
    let quoted = core::QuotedLine {
        amount_subtotal: line.amount_subtotal,
        taxable_amount: line.taxable_amount,
        tax_rate: line.tax_rate,
        unit_price: line.unit_price,
        sub_lines: line
            .sub_lines
            .iter()
            .map(|sub_line| {
                let (flat_cap, flat_fee) = match &sub_line.attributes {
                    Some(
                        SubLineAttributes::Tiered {
                            flat_cap, flat_fee, ..
                        }
                        | SubLineAttributes::Volume {
                            flat_cap, flat_fee, ..
                        },
                    ) => (*flat_cap, *flat_fee),
                    Some(SubLineAttributes::Package { .. } | SubLineAttributes::Matrix { .. })
                    | None => (None, None),
                };
                core::SubLinePrice {
                    total: sub_line.total,
                    unit_price: sub_line.unit_price,
                    flat_cap,
                    flat_fee,
                }
            })
            .collect(),
    };
    let Some(net) = core::net_line(&quoted, quoted_taxable) else {
        return 0;
    };

    line.tax_inclusive_quote = Some(TaxInclusiveQuote {
        amount_subtotal: line.amount_subtotal,
        unit_price: line.unit_price,
        sub_lines: line.sub_lines.clone(),
    });
    line.amount_subtotal = net.amount_subtotal;
    line.unit_price = net.unit_price;
    for (sub_line, price) in line.sub_lines.iter_mut().zip(net.sub_lines) {
        sub_line.total = price.total;
        sub_line.unit_price = price.unit_price;
        if let Some(
            SubLineAttributes::Tiered {
                flat_cap, flat_fee, ..
            }
            | SubLineAttributes::Volume {
                flat_cap, flat_fee, ..
            },
        ) = &mut sub_line.attributes
        {
            *flat_cap = price.flat_cap;
            *flat_fee = price.flat_fee;
        }
    }
    net.discount_tax
}

/// Shares the netted invoice discount back over the coupons that made it up, so the coupon
/// rows a document lists still add up to the discount it shows. What each coupon took off
/// the quoted price is kept as `quoted_value`: that, not its net share, is what the
/// customer used up of a fixed-amount coupon.
/// `net` is each coupon's share (`billing_core::assembly::net_discount`), None when unchanged.
pub(crate) fn net_coupons(coupons: &mut [CouponLineItem], net: Option<Vec<i64>>) {
    let Some(net) = net else {
        return;
    };
    for (coupon, net_value) in coupons.iter_mut().zip(net) {
        coupon.quoted_value = Some(coupon.value);
        coupon.value = net_value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coupons::CouponDiscount;
    use crate::domain::{LineItemKind, SubLineItem};
    use chrono::NaiveDate;
    use common_domain::ids::{AppliedCouponId, BaseId, CouponId};
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    fn line(amount_subtotal: i64, taxable_amount: i64, unit_price: Decimal) -> LineItem {
        let day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        LineItem {
            local_id: "line".to_string(),
            name: "Seats".to_string(),
            charge_type: None,
            amount_subtotal,
            tax_rate: dec!(0.2),
            taxable_amount,
            tax_amount: 0,
            amount_total: 0,
            tax_details: vec![],
            quantity: Some(dec!(1)),
            unit_price: Some(unit_price),
            start_date: day,
            end_date: day,
            sub_lines: vec![],
            is_prorated: false,
            price_component_id: None,
            sub_component_id: None,
            sub_add_on_id: None,
            product_id: None,
            metric_id: None,
            description: None,
            group_by_dimensions: None,
            kind: LineItemKind::Regular,
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

    fn coupon(value: i64) -> CouponLineItem {
        CouponLineItem {
            coupon_id: CouponId::new(),
            applied_coupon_id: AppliedCouponId::new(),
            name: "Coupon".to_string(),
            code: "CODE".to_string(),
            value,
            quoted_value: None,
            discount: CouponDiscount::Percentage(dec!(10)),
        }
    }

    #[test]
    fn an_undiscounted_line_takes_the_engines_net_and_keeps_its_quote() {
        // 9.99 quoted at 20%: the engine carved 1.67 out and reported 8.32.
        let mut item = line(999, 832, dec!(9.99));

        assert_eq!(net_line(&mut item, 999), 0);
        assert_eq!(item.amount_subtotal, 832);
        assert_eq!(item.unit_price, Some(dec!(8.32)));
        let quote = item
            .tax_inclusive_quote
            .expect("the quoted figures are kept");
        assert_eq!(quote.amount_subtotal, 999);
        assert_eq!(quote.unit_price, Some(dec!(9.99)));
    }

    #[test]
    fn a_discounted_line_reports_the_tax_that_was_in_its_discount() {
        // 100.00 quoted, 10.00 off: 90.00 taxable, of which 75.00 is net at 20%.
        let mut item = line(10000, 7500, dec!(100));

        let discount_tax = net_line(&mut item, 9000);
        // The subtotal nets to 83.33, so the discount nets from 10.00 to 8.33.
        assert_eq!(item.amount_subtotal, 8333);
        assert_eq!(discount_tax, 1000 - 833);
    }

    #[test]
    fn a_line_that_bore_no_tax_stays_as_quoted() {
        let mut item = line(999, 999, dec!(9.99));

        assert_eq!(net_line(&mut item, 999), 0);
        assert_eq!(item.amount_subtotal, 999);
        assert_eq!(item.tax_inclusive_quote, None);
    }

    #[test]
    fn a_line_left_nothing_to_tax_nets_at_its_rate() {
        // 12.00 quoted at 20%, all of it taken off by a discount or promotional credit.
        let mut item = line(1200, 0, dec!(12));
        assert_eq!(net_line(&mut item, 0), 200);
        assert_eq!(item.amount_subtotal, 1000);
        assert_eq!(
            item.tax_inclusive_quote.map(|quote| quote.amount_subtotal),
            Some(1200)
        );
    }

    #[test]
    fn sub_lines_keep_adding_up_to_their_line() {
        let mut item = line(1000, 833, dec!(10));
        item.sub_lines = vec![
            SubLineItem {
                local_id: "a".to_string(),
                name: "Tier 1".to_string(),
                total: 333,
                quantity: dec!(1),
                unit_price: dec!(3.33),
                attributes: None,
            },
            SubLineItem {
                local_id: "b".to_string(),
                name: "Tier 2".to_string(),
                total: 667,
                quantity: dec!(1),
                unit_price: dec!(6.67),
                attributes: None,
            },
        ];

        net_line(&mut item, 1000);
        assert_eq!(item.sub_lines.iter().map(|s| s.total).sum::<i64>(), 833);
    }

    #[test]
    fn netting_round_trips_through_the_quote() {
        let quoted = line(999, 999, dec!(9.99));
        let mut netted = LineItem {
            taxable_amount: 832,
            ..quoted.clone()
        };
        net_line(&mut netted, 999);

        assert_eq!(
            netted.into_tax_inclusive_quote(),
            quoted_as_repriced(quoted)
        );
    }

    fn quoted_as_repriced(mut quoted: LineItem) -> LineItem {
        quoted.amount_total = quoted.amount_subtotal;
        quoted
    }

    #[test]
    fn netted_coupons_add_up_to_the_netted_discount() {
        let mut coupons = vec![coupon(500), coupon(500)];

        net_coupons(&mut coupons, core::net_coupons(&[500, 500], 833));
        assert_eq!(coupons.iter().map(|c| c.value).sum::<i64>(), 833);
        assert!(coupons.iter().all(|c| c.quoted_value == Some(500)));
    }
}
