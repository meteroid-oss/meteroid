//! A document put together from its priced lines: what an earlier estimate took off them put
//! back, the coupons (R-020, R-089) spread over them (R-070), and once the store resolved their
//! taxes, the discount restated net (R-031).

use crate::assembly::discount::{
    Coupon, CouponDiscount, calculate_coupons_discount, distribute_discount,
};
use crate::assembly::tax_inclusive::net_coupons;
use crate::credit::allocation::Overflow;
use crate::document::Line;

/// What assembling a document reads beyond its lines.
#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyInputs {
    pub currency: String,
    /// The coupons applicable to this document.
    pub coupons: Vec<Coupon>,
    /// A discount given as an amount (a one-off invoice), instead of coupons.
    pub discount: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembled {
    /// Each line's taxable amount after the discount, before tax.
    pub lines: Vec<Line>,
    /// What each coupon took off, in the order they apply.
    pub coupons: Vec<CouponDiscount>,
    pub discount: u64,
}

/// The lines with the discount off their taxable amounts, ready to be taxed. A line kept from an
/// earlier estimate gets back what that estimate took off it, so assembling is idempotent.
pub fn assemble(lines: Vec<Line>, inputs: &AssemblyInputs) -> Assembled {
    let mut lines = lines;
    // The discount is spread afresh: a line kept from an earlier estimate keeps none of its share.
    for line in &mut lines {
        line.promotional = None;
        line.taxable = line.figures.subtotal;
    }
    let (coupons, discount) = match inputs.discount {
        Some(discount) => (Vec::new(), discount),
        None => {
            let subtotal = lines
                .iter()
                .fold(0, |sum, line| sum + line.figures.subtotal);
            let coupons = calculate_coupons_discount(subtotal, &inputs.currency, &inputs.coupons);
            let discount = coupons.iter().map(|coupon| coupon.value).sum::<i64>();
            (coupons, u64::try_from(discount).unwrap_or(0))
        }
    };
    let subtotals: Vec<i64> = lines.iter().map(|line| line.figures.subtotal).collect();
    for (line, taxable) in lines
        .iter_mut()
        .zip(distribute_discount(&subtotals, discount))
    {
        if let Some(taxable) = taxable {
            line.taxable = taxable;
        }
    }
    Assembled {
        lines,
        coupons,
        discount,
    }
}

/// The document discount restated to match lines the tax pass netted (R-031): less the part of
/// it that was tax, the promotional credit's own tax staying with it; and the coupons' values
/// shared over it, None when they already add up to it.
pub fn net_discount(
    discount: i64,
    discount_tax: i64,
    lines: &[Line],
    coupons: &[i64],
) -> Result<(i64, Option<Vec<i64>>), Overflow> {
    let sum = |amount: fn(&Line) -> i64| {
        lines
            .iter()
            .try_fold(0_i64, |sum, line| sum.checked_add(amount(line)))
            .ok_or(Overflow)
    };
    let quoted = sum(|line| line.promotional.unwrap_or(0))?;
    let net = sum(Line::net_promotional)?;
    let net_discount = discount - (discount_tax - (quoted - net));
    Ok((net_discount, net_coupons(coupons, net_discount)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::discount::CouponValue;
    use crate::document::{Figures, LineKind};
    use rust_decimal::Decimal;
    use uuid::Uuid;

    fn line(amount: i64) -> Line {
        Line {
            id: amount.to_string(),
            kind: LineKind::Fee,
            subscription: None,
            product: None,
            recurring: true,
            figures: Figures {
                subtotal: amount,
                unit_price: None,
                sub_lines: Vec::new(),
            },
            quote: None,
            taxable: amount,
            tax: 0,
            total: amount,
            tax_rate: Decimal::ZERO,
            taxes: Vec::new(),
            promotional: None,
            taxed_at_redemption: false,
        }
    }

    fn inputs(coupons: Vec<Coupon>) -> AssemblyInputs {
        AssemblyInputs {
            currency: "USD".to_owned(),
            coupons,
            discount: None,
        }
    }

    /// 10% then 5.00 off 60.00 and 40.00: 15.00 off, spread 9.00 and 6.00.
    #[test]
    fn coupons_apply_in_order_and_spread_over_the_lines() {
        let coupons = vec![
            Coupon {
                coupon_id: Uuid::from_u128(1).into(),
                value: CouponValue::Fixed {
                    amount: Decimal::from(5),
                    currency: "USD".to_owned(),
                    exponent: 2,
                },
            },
            Coupon {
                coupon_id: Uuid::from_u128(2).into(),
                value: CouponValue::Percentage(Decimal::from(10)),
            },
        ];
        let assembled = assemble(vec![line(6_000), line(4_000)], &inputs(coupons));
        assert_eq!(assembled.discount, 1_500);
        assert_eq!(
            assembled
                .coupons
                .iter()
                .map(|c| (c.coupon, c.value))
                .collect::<Vec<_>>(),
            vec![(1, 1_000), (0, 500)]
        );
        assert_eq!(
            assembled
                .lines
                .iter()
                .map(|l| l.taxable)
                .collect::<Vec<_>>(),
            vec![5_100, 3_400]
        );
    }

    /// A line kept from an earlier estimate is assembled as if it were priced afresh.
    #[test]
    fn assembling_is_idempotent() {
        let first = assemble(vec![line(1_000)], &inputs(Vec::new()));
        let mut kept = first.lines.clone();
        kept[0].taxable -= 300;
        kept[0].promotional = Some(300);
        assert_eq!(assemble(kept, &inputs(Vec::new())), first);
    }

    /// A line kept from a draft a 25 % coupon discounted, 5000 taxable 3750, assembled once the
    /// coupon is no longer the invoice's: taxable 5000 again, none of the earlier share left.
    #[test]
    fn a_kept_line_sheds_a_discount_the_invoice_no_longer_takes() {
        let discounted = assemble(
            vec![line(5_000)],
            &inputs(vec![Coupon {
                coupon_id: Uuid::from_u128(1).into(),
                value: CouponValue::Percentage(Decimal::from(25)),
            }]),
        );
        assert_eq!(discounted.lines[0].taxable, 3_750);
        let assembled = assemble(discounted.lines, &inputs(Vec::new()));
        assert_eq!(assembled.discount, 0);
        assert_eq!(assembled.lines[0].taxable, 5_000);
    }

    #[test]
    fn a_tax_inclusive_discount_is_restated_net() {
        // 10.00 off quoted prices, of which 1.67 was tax: 8.33 off the net lines.
        let (discount, coupons) = net_discount(1_000, 167, &[line(8_333)], &[500, 500]).unwrap();
        assert_eq!(discount, 833);
        assert_eq!(coupons.map(|c| c.iter().sum::<i64>()), Some(833));
    }
}
