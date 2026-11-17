//! Turns lines priced tax-included into the net lines an invoice stores.
//!
//! A tax-inclusive plan quotes the customer a price that already contains the tax. The
//! whole pricing pipeline — fees, proration, coupons, the minimum commitment — runs on
//! those quoted figures, exactly as it does for a net plan: a "5.00 off" coupon takes 5.00
//! off the price the customer sees, and a commitment is a commitment to spend that much.
//! Only once the tax engine has carved the tax out of each line is the invoice rewritten
//! in net terms, here, so that `subtotal - discount + tax = total` holds the way every
//! reader of an invoice expects, and the total is still the quoted price to the unit.

use rust_decimal::Decimal;

use crate::allocation::largest_remainder;
use crate::rounding::round_to_minor;

/// Decimal places kept on a netted unit price: enough that `unit_price * quantity` lands
/// back on the netted subtotal, without carrying a division's full expansion into storage.
const UNIT_PRICE_SCALE: u32 = 10;

/// The figures of a sub-line that net; `flat_cap` and `flat_fee` only on tiered and volume ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubLinePrice {
    pub total: i64,
    pub unit_price: Decimal,
    pub flat_cap: Option<Decimal>,
    pub flat_fee: Option<Decimal>,
}

/// A line as priced, tax included, with the net taxable amount the engine reported.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotedLine {
    pub amount_subtotal: i64,
    pub taxable_amount: i64,
    pub tax_rate: Decimal,
    pub unit_price: Option<Decimal>,
    pub sub_lines: Vec<SubLinePrice>,
}

/// The line's net figures, and how much of its discount was tax.
#[derive(Clone, Debug, PartialEq)]
pub struct NetLine {
    pub amount_subtotal: i64,
    pub unit_price: Option<Decimal>,
    pub sub_lines: Vec<SubLinePrice>,
    pub discount_tax: i64,
}

/// Rewrites a line in net terms once the engine has carved the tax out of it (R-031).
///
/// `quoted_taxable` is the line's taxable amount as priced (tax included, after discount);
/// `line.taxable_amount` is already the net the engine reported. `None` when the line stays
/// exactly as priced (a reverse-charge, exempt or export customer pays the quoted price in full),
/// its discount bearing no tax.
pub fn net_line(line: &QuotedLine, quoted_taxable: i64) -> Option<NetLine> {
    let net_taxable = line.taxable_amount;
    let quoted_subtotal = line.amount_subtotal;
    // The subtotal nets in the same proportion its taxable amount did. An undiscounted
    // line *is* its taxable amount, and takes the engine's figure rather than a second
    // rounding of it. A line left nothing to tax nets at its own rate.
    let net_subtotal = if quoted_taxable == 0 {
        if line.tax_rate.is_zero() || quoted_subtotal == 0 {
            return None;
        }
        round_to_minor(Decimal::from(quoted_subtotal) / (Decimal::ONE + line.tax_rate))
    } else if net_taxable == quoted_taxable {
        return None;
    } else if quoted_subtotal == quoted_taxable {
        net_taxable
    } else {
        scale(quoted_subtotal, net_taxable, quoted_taxable)
    };

    let mut unit_price = line.unit_price;
    let mut sub_lines = line.sub_lines.clone();
    if quoted_subtotal != 0 {
        let ratio = Decimal::from(net_subtotal) / Decimal::from(quoted_subtotal);
        unit_price = unit_price.map(|price| net_price(price, ratio));
        net_sub_lines(&mut sub_lines, ratio);
    }

    Some(NetLine {
        amount_subtotal: net_subtotal,
        unit_price,
        sub_lines,
        discount_tax: (quoted_subtotal - quoted_taxable) - (net_subtotal - net_taxable),
    })
}

/// The coupons' values, tax included, shared back over the netted invoice discount; `None`
/// when they already add up to it.
pub fn net_coupons(quoted: &[i64], net_discount: i64) -> Option<Vec<i64>> {
    let quoted_discount: i64 = quoted.iter().sum();
    if quoted_discount == net_discount || quoted_discount == 0 {
        return None;
    }

    let ratio = Decimal::from(net_discount) / Decimal::from(quoted_discount);
    let exact: Vec<Decimal> = quoted
        .iter()
        .map(|value| Decimal::from(*value) * ratio)
        .collect();
    Some(largest_remainder(&exact))
}

fn scale(amount: i64, numerator: i64, denominator: i64) -> i64 {
    round_to_minor(Decimal::from(amount) * Decimal::from(numerator) / Decimal::from(denominator))
}

fn net_price(price: Decimal, ratio: Decimal) -> Decimal {
    (price * ratio).round_dp(UNIT_PRICE_SCALE).normalize()
}

/// Nets the sub-lines as one group, so they keep adding up to the line they detail.
fn net_sub_lines(sub_lines: &mut [SubLinePrice], ratio: Decimal) {
    let exact: Vec<Decimal> = sub_lines
        .iter()
        .map(|sub_line| Decimal::from(sub_line.total) * ratio)
        .collect();

    for (sub_line, total) in sub_lines.iter_mut().zip(largest_remainder(&exact)) {
        sub_line.total = total;
        sub_line.unit_price = net_price(sub_line.unit_price, ratio);
        sub_line.flat_cap = sub_line.flat_cap.map(|cap| net_price(cap, ratio));
        sub_line.flat_fee = sub_line.flat_fee.map(|fee| net_price(fee, ratio));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn line(amount_subtotal: i64, taxable_amount: i64, unit_price: Decimal) -> QuotedLine {
        QuotedLine {
            amount_subtotal,
            taxable_amount,
            tax_rate: dec!(0.2),
            unit_price: Some(unit_price),
            sub_lines: vec![],
        }
    }

    #[test]
    fn an_undiscounted_line_takes_the_engines_net() {
        // 9.99 quoted at 20%: the engine carved 1.67 out and reported 8.32.
        let net = net_line(&line(999, 832, dec!(9.99)), 999).expect("netted");
        assert_eq!(net.discount_tax, 0);
        assert_eq!(net.amount_subtotal, 832);
        assert_eq!(net.unit_price, Some(dec!(8.32)));
    }

    #[test]
    fn a_discounted_line_reports_the_tax_that_was_in_its_discount() {
        // 100.00 quoted, 10.00 off: 90.00 taxable, of which 75.00 is net at 20%.
        let net = net_line(&line(10000, 7500, dec!(100)), 9000).expect("netted");
        // The subtotal nets to 83.33, so the discount nets from 10.00 to 8.33.
        assert_eq!(net.amount_subtotal, 8333);
        assert_eq!(net.discount_tax, 1000 - 833);
    }

    #[test]
    fn a_line_that_bore_no_tax_stays_as_quoted() {
        assert_eq!(net_line(&line(999, 999, dec!(9.99)), 999), None);
    }

    #[test]
    fn a_line_left_nothing_to_tax_nets_at_its_rate() {
        // 12.00 quoted at 20%, all of it taken off by a discount or promotional credit.
        let net = net_line(&line(1200, 0, dec!(12)), 0).expect("netted");
        assert_eq!(net.discount_tax, 200);
        assert_eq!(net.amount_subtotal, 1000);
    }

    #[test]
    fn sub_lines_keep_adding_up_to_their_line() {
        let mut item = line(1000, 833, dec!(10));
        let sub_line = |total, unit_price| SubLinePrice {
            total,
            unit_price,
            flat_cap: None,
            flat_fee: None,
        };
        item.sub_lines = vec![sub_line(333, dec!(3.33)), sub_line(667, dec!(6.67))];

        let net = net_line(&item, 1000).expect("netted");
        assert_eq!(net.sub_lines.iter().map(|s| s.total).sum::<i64>(), 833);
    }

    #[test]
    fn netted_coupons_add_up_to_the_netted_discount() {
        let net = net_coupons(&[500, 500], 833).expect("netted");
        assert_eq!(net.iter().sum::<i64>(), 833);
        assert_eq!(net_coupons(&[500, 500], 1000), None);
    }
}
