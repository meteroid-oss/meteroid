//! The single place tax is turned into money: how a rate is applied to a base, how the
//! result is rounded, and how the rounded total is shared back out over the lines.
//!
//! Resolving *which* rates apply is the engine's job and lives elsewhere; everything that
//! already knows its rates — the engine, the draft editor, manual invoices — applies them
//! through here, so a rounding rule only ever has to be fixed once.

use ordermap::OrderMap;
use rust_decimal::Decimal;

/// One tax to levy on a line.
#[derive(Debug, Clone)]
pub struct TaxToApply {
    pub rate: Decimal,
    /// What makes two taxes the same tax across lines, and so one rounding group and one
    /// breakdown row: an accounting code where there is one, else the name and rate.
    pub key: String,
    /// Levied on the base plus the tax accrued before it (e.g. Canadian QST on top of GST).
    pub compound: bool,
}

/// A line's taxable base and the taxes levied on it, in the order they stack.
#[derive(Debug, Clone)]
pub struct LineToTax {
    /// The net base — or, on a tax-inclusive line, the gross amount the taxes are carved
    /// out of.
    pub taxable_amount: i64,
    /// `taxable_amount` already contains the taxes. The gross is what the customer was
    /// quoted, so it is kept to the unit: the tax is rounded once, and the net base is
    /// whatever is left ([`LineToTax::net_amount`]).
    pub tax_inclusive: bool,
    pub taxes: Vec<TaxToApply>,
}

impl LineToTax {
    /// The line's net base given the amounts [`apply_taxes`] allocated to it.
    ///
    /// On an inclusive line it is derived by subtraction rather than division, so net and
    /// tax always add back to the gross. The price of that guarantee is that the tax may
    /// sit one unit away from `net × rate` — a gross like 10.05 at 20% has no net that
    /// reproduces it — which is within the tolerance EN 16931 (BR-CO-17) allows.
    pub fn net_amount(&self, allocated: &[i64]) -> i64 {
        if self.tax_inclusive {
            self.taxable_amount - allocated.iter().sum::<i64>()
        } else {
            self.taxable_amount
        }
    }
}

/// Applies every line's taxes and rounds each tax once over its whole taxable base,
/// returning the minor units allocated to each line's taxes, in the order given.
///
/// Rounding per line instead accumulates drift, so the invoice shows a cent less than the
/// rate applied to its own subtotal — which is the figure a customer checks.
pub fn apply_taxes(lines: &[LineToTax]) -> Vec<Vec<i64>> {
    let exact: Vec<Vec<Decimal>> = lines.iter().map(exact_amounts).collect();

    let mut groups: OrderMap<&str, Vec<(usize, usize)>> = OrderMap::new();
    for (line_index, line) in lines.iter().enumerate() {
        for (tax_index, tax) in line.taxes.iter().enumerate() {
            groups
                .entry(tax.key.as_str())
                .or_default()
                .push((line_index, tax_index));
        }
    }

    let mut allocated: Vec<Vec<i64>> = exact.iter().map(|amounts| vec![0; amounts.len()]).collect();

    for members in groups.into_values() {
        let shares: Vec<Decimal> = members
            .iter()
            .map(|(line_index, tax_index)| exact[*line_index][*tax_index])
            .collect();

        for (&(line_index, tax_index), amount) in members.iter().zip(largest_remainder(&shares)) {
            allocated[line_index][tax_index] = amount;
        }
    }

    allocated
}

/// The exact, unrounded amount of each tax on a line. Compound taxes levy on the base plus
/// the tax accrued so far, so the chain is built before anything is rounded.
///
/// An inclusive line runs the same chain on a base of one to learn how much tax each unit
/// of net carries, then scales it to the net its gross implies. Dividing by the sum of the
/// rates instead would overstate the net as soon as one of them compounds.
fn exact_amounts(line: &LineToTax) -> Vec<Decimal> {
    let amount = Decimal::from(line.taxable_amount);
    if !line.tax_inclusive {
        return tax_chain(amount, &line.taxes);
    }

    let per_unit = tax_chain(Decimal::ONE, &line.taxes);
    let gross_per_unit = Decimal::ONE + per_unit.iter().sum::<Decimal>();
    if gross_per_unit <= Decimal::ZERO {
        // A chain of rates at or below -100% has no net to recover.
        return vec![Decimal::ZERO; line.taxes.len()];
    }

    let net = amount / gross_per_unit;
    per_unit.into_iter().map(|share| net * share).collect()
}

/// The rate a tax-inclusive price actually carries: how much tax one unit of net bears,
/// once compounding is accounted for. A chain is not the sum of its rates as soon as one
/// of its taxes levies on the tax before it, so anything that has to divide a gross price
/// back down asks here rather than adding the rates up itself.
pub fn effective_rate(taxes: &[TaxToApply]) -> Decimal {
    tax_chain(Decimal::ONE, taxes).iter().sum()
}

fn tax_chain(base: Decimal, taxes: &[TaxToApply]) -> Vec<Decimal> {
    let mut running = Decimal::ZERO;

    taxes
        .iter()
        .map(|tax| {
            let taxable = if tax.compound { base + running } else { base };
            let amount = taxable * tax.rate;
            running += amount;
            amount
        })
        .collect()
}

pub use billing_core::{largest_remainder, round_to_minor};

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn tax(rate: Decimal, key: &str, compound: bool) -> TaxToApply {
        TaxToApply {
            rate,
            key: key.to_string(),
            compound,
        }
    }

    #[test]
    fn keeps_the_rounded_total_and_favours_the_largest_remainder() {
        // 7.56 € at 19%, split across two lines: 1.44 € owed, the cent goes to 0.44.
        assert_eq!(
            largest_remainder(&[dec!(14.44), dec!(129.20)]),
            vec![15, 129]
        );
        // Ten shares that each round up on their own still total the 66 owed on 3.30 €.
        let shares = vec![dec!(6.6); 10];
        assert_eq!(largest_remainder(&shares).iter().sum::<i64>(), 66);
    }

    #[test]
    fn handles_credit_and_mixed_sign_shares() {
        assert_eq!(largest_remainder(&[dec!(-14.44)]), vec![-14]);
        assert_eq!(
            largest_remainder(&[dec!(10.5), dec!(-3.2)])
                .iter()
                .sum::<i64>(),
            7
        );
    }

    #[test]
    fn never_charges_a_zero_share() {
        assert_eq!(
            largest_remainder(&[dec!(0), dec!(0.6), dec!(0.6)]),
            vec![0, 1, 0]
        );
    }

    #[test]
    fn a_rate_is_rounded_once_across_the_lines_that_carry_it() {
        let lines = vec![
            LineToTax {
                tax_inclusive: false,
                taxable_amount: 1512,
                taxes: vec![tax(dec!(0.2), "vat", false)],
            },
            LineToTax {
                tax_inclusive: false,
                taxable_amount: 1017,
                taxes: vec![tax(dec!(0.2), "vat", false)],
            },
        ];

        // 25.29 at 20% is 5.058; per line it would be 3.02 + 2.03 = 5.05.
        assert_eq!(apply_taxes(&lines), vec![vec![303], vec![203]]);
    }

    #[test]
    fn each_key_is_rounded_over_its_own_base() {
        let lines = vec![LineToTax {
            tax_inclusive: false,
            taxable_amount: 1017,
            taxes: vec![tax(dec!(0.2), "vat", false), tax(dec!(0.055), "eco", false)],
        }];

        assert_eq!(apply_taxes(&lines), vec![vec![203, 56]]);
    }

    #[test]
    fn a_compound_tax_levies_on_the_base_plus_the_tax_before_it() {
        let lines = vec![LineToTax {
            tax_inclusive: false,
            taxable_amount: 10000,
            taxes: vec![
                tax(dec!(0.05), "gst", false),
                tax(dec!(0.09975), "qst", true),
            ],
        }];

        // QST is 9.975% of 105.00, not of 100.00: 10.47375 -> 10.47.
        assert_eq!(apply_taxes(&lines), vec![vec![500, 1047]]);
    }

    fn inclusive(taxable_amount: i64, taxes: Vec<TaxToApply>) -> LineToTax {
        LineToTax {
            taxable_amount,
            tax_inclusive: true,
            taxes,
        }
    }

    #[test]
    fn an_inclusive_line_carves_the_tax_out_of_its_gross() {
        let lines = vec![inclusive(1200, vec![tax(dec!(0.2), "vat", false)])];

        let allocated = apply_taxes(&lines);
        assert_eq!(allocated, vec![vec![200]]);
        assert_eq!(lines[0].net_amount(&allocated[0]), 1000);
    }

    #[test]
    fn an_inclusive_gross_survives_a_net_that_cannot_reproduce_it() {
        // No net gives 10.05 at 20%: 8.37 makes 10.04 and 8.38 makes 10.06. The gross is
        // the quoted price, so it holds and the net absorbs the unit.
        let lines = vec![inclusive(1005, vec![tax(dec!(0.2), "vat", false)])];

        let allocated = apply_taxes(&lines);
        let net = lines[0].net_amount(&allocated[0]);
        assert_eq!(allocated, vec![vec![168]]);
        assert_eq!(net, 837);
        assert_eq!(net + allocated[0][0], 1005);
    }

    #[test]
    fn an_inclusive_rate_is_rounded_once_across_its_lines() {
        let lines = vec![
            inclusive(999, vec![tax(dec!(0.2), "vat", false)]),
            inclusive(999, vec![tax(dec!(0.2), "vat", false)]),
            inclusive(999, vec![tax(dec!(0.2), "vat", false)]),
        ];

        // 29.97 carries 4.995 of tax: 5.00 over the invoice, where three lines rounded
        // alone (1.665 each, so 1.67) would add up to 5.01.
        let allocated = apply_taxes(&lines);
        assert_eq!(allocated.iter().flatten().sum::<i64>(), 500);
        for (line, amounts) in lines.iter().zip(&allocated) {
            assert_eq!(line.net_amount(amounts) + amounts[0], 999);
        }
    }

    #[test]
    fn an_inclusive_compound_chain_divides_through_the_whole_stack() {
        // 100.00 net bears 5.00 GST and 10.47375 QST, so 114.97 gross is that line seen
        // from the other side (the half-cent the exclusive side rounds away aside).
        let lines = vec![inclusive(
            11547,
            vec![
                tax(dec!(0.05), "gst", false),
                tax(dec!(0.09975), "qst", true),
            ],
        )];

        let allocated = apply_taxes(&lines);
        assert_eq!(allocated, vec![vec![500, 1047]]);
        assert_eq!(lines[0].net_amount(&allocated[0]), 10000);
    }

    #[test]
    fn an_inclusive_credit_gives_its_tax_back() {
        let lines = vec![inclusive(-1200, vec![tax(dec!(0.2), "vat", false)])];

        let allocated = apply_taxes(&lines);
        assert_eq!(allocated, vec![vec![-200]]);
        assert_eq!(lines[0].net_amount(&allocated[0]), -1000);
    }

    #[test]
    fn the_effective_rate_of_a_chain_includes_its_compounding() {
        assert_eq!(effective_rate(&[]), dec!(0));
        assert_eq!(effective_rate(&[tax(dec!(0.2), "vat", false)]), dec!(0.2));
        // GST 5% then QST 9.975% on top of it: 0.05 + 0.09975 * 1.05.
        assert_eq!(
            effective_rate(&[
                tax(dec!(0.05), "gst", false),
                tax(dec!(0.09975), "qst", true),
            ]),
            dec!(0.1547375)
        );
    }

    #[test]
    fn an_inclusive_line_with_no_tax_keeps_its_whole_amount() {
        let lines = vec![inclusive(999, vec![])];

        let allocated = apply_taxes(&lines);
        assert_eq!(lines[0].net_amount(&allocated[0]), 999);
    }
}
