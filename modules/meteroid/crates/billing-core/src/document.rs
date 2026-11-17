//! A document's lines as assembly and credit settlement read and rewrite them: their money
//! figures, the charge credit may pay, and the rates they are taxed by. The store converts its own
//! lines at the boundary; everything here is keyed by the line's id.

use chrono::{DateTime, Utc};
use common_domain::ids::{ProductId, SubscriptionId};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::assembly::tax_inclusive::{QuotedLine, SubLinePrice, net_line};
use crate::rounding::round_to_minor;
use crate::tax::{LineToTax, TaxToApply, apply_taxes};
use crate::totals::{LineAmounts, Totals};

/// What a line bills, as credit reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// A fee, a shortfall, or anything else that is not metered usage.
    Fee,
    /// Metered usage consumed before `dated_before`: only credit effective before it pays it.
    Usage { dated_before: DateTime<Utc> },
    /// A credit purchase: no credit pays it (R4).
    Credit,
}

/// The figures a line is priced by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Figures {
    pub subtotal: i64,
    pub unit_price: Option<Decimal>,
    pub sub_lines: Vec<SubLinePrice>,
}

/// A tax a line levies, and what it came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineTax {
    pub rate: Decimal,
    pub name: String,
    pub amount: i64,
}

/// A rate a line is taxed by: one rounding group per name and rate over the document (R-030).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rate {
    pub rate: Decimal,
    pub name: String,
}

impl Rate {
    fn to_apply(&self) -> TaxToApply {
        TaxToApply {
            rate: self.rate,
            key: format!("{}:{}", self.name, self.rate),
            compound: false,
        }
    }
}

/// Each line's rates, in line order: resolved by the store, applied here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rates(pub Vec<Vec<Rate>>);

impl Rates {
    pub fn of(&self, index: usize) -> &[Rate] {
        self.0.get(index).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub kind: LineKind,
    /// The subscription it bills, as the store resolved it.
    pub subscription: Option<SubscriptionId>,
    /// The product credit restricted to products answers to.
    pub product: Option<ProductId>,
    /// It bills no metered usage (R-076).
    pub recurring: bool,
    /// As stored: net on a tax-inclusive line that bore tax.
    pub figures: Figures,
    /// The tax-included figures a netted line was quoted in (R-031).
    pub quote: Option<Figures>,
    pub taxable: i64,
    pub tax: i64,
    pub total: i64,
    pub tax_rate: Decimal,
    pub taxes: Vec<LineTax>,
    /// Promotional credit taken off it before tax, in the figures it was quoted in.
    pub promotional: Option<i64>,
    /// A credit purchase whose tax is due when the credit is used (R-032).
    pub taxed_at_redemption: bool,
}

impl Line {
    /// The document figure of its promotional credit: the net share of its reduction on a netted
    /// line, the quoted share otherwise.
    pub fn net_promotional(&self) -> i64 {
        let quoted = self.promotional.unwrap_or(0);
        let Some(quote) = self.quote.as_ref().filter(|_| quoted != 0) else {
            return quoted;
        };
        let reduced_quoted = quote.subtotal - self.total;
        let reduced_net = self.figures.subtotal - self.taxable;
        if reduced_quoted <= 0 {
            return quoted;
        }
        round_to_minor(
            Decimal::from(reduced_net) * Decimal::from(quoted) / Decimal::from(reduced_quoted),
        )
    }

    /// Back in the figures it was priced in, ready to be taxed again; a line without its quote is
    /// grossed up by its own rate.
    pub fn into_quote(mut self) -> Line {
        if let Some(quote) = self.quote.take() {
            self.figures = quote;
        } else if !self.tax_rate.is_zero() {
            let factor = Decimal::ONE + self.tax_rate;
            let gross_up = |amount: i64| {
                (Decimal::from(amount) * factor)
                    .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
                    .to_i64()
                    .unwrap_or(amount)
            };
            self.figures.subtotal = gross_up(self.figures.subtotal);
            self.figures.unit_price = self.figures.unit_price.map(|price| price * factor);
            for sub_line in &mut self.figures.sub_lines {
                sub_line.total = gross_up(sub_line.total);
                sub_line.unit_price *= factor;
            }
        }
        self.taxable = self.figures.subtotal;
        self.total = self.figures.subtotal;
        self.tax = 0;
        self.promotional = None;
        self
    }

    /// Rewrites it net once its tax is carved out (R-031); `quoted_taxable` is its taxable amount
    /// as priced. Returns how much of its discount was tax, None when it stays as priced.
    pub fn net(&mut self, quoted_taxable: i64) -> Option<i64> {
        let quoted = QuotedLine {
            amount_subtotal: self.figures.subtotal,
            taxable_amount: self.taxable,
            tax_rate: self.tax_rate,
            unit_price: self.figures.unit_price,
            sub_lines: self.figures.sub_lines.clone(),
        };
        let net = net_line(&quoted, quoted_taxable)?;
        self.quote = Some(self.figures.clone());
        self.figures = Figures {
            subtotal: net.amount_subtotal,
            unit_price: net.unit_price,
            sub_lines: net.sub_lines,
        };
        Some(net.discount_tax)
    }

    pub fn amounts(&self) -> LineAmounts {
        LineAmounts {
            subtotal: self.figures.subtotal,
            tax: self.tax,
            promotional: self.net_promotional(),
            recurring: self.recurring,
        }
    }
}

/// What applying rates to lines did: the tax in their discounts, and which lines were netted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub discount_tax: i64,
    pub netted: Vec<bool>,
}

/// Taxes each line on its taxable amount by its rates, each rate rounded once over the lines
/// (R-030), and nets the tax-inclusive ones (R-031).
pub fn apply_rates(lines: &mut [Line], rates: &[&[Rate]], tax_inclusive: bool) -> Applied {
    let to_apply: Vec<LineToTax> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| LineToTax {
            taxable_amount: line.taxable,
            tax_inclusive,
            taxes: rates
                .get(index)
                .copied()
                .unwrap_or_default()
                .iter()
                .map(Rate::to_apply)
                .collect(),
        })
        .collect();
    let rate_of = |index: usize| rates.get(index).copied().unwrap_or_default();

    let mut applied = Applied {
        discount_tax: 0,
        netted: vec![false; lines.len()],
    };
    for (index, ((line, taxes), amounts)) in lines
        .iter_mut()
        .zip(to_apply.iter())
        .zip(apply_taxes(&to_apply))
        .enumerate()
    {
        line.taxes = rate_of(index)
            .iter()
            .zip(amounts.iter().copied())
            .map(|(rate, amount)| LineTax {
                rate: rate.rate,
                name: rate.name.clone(),
                amount,
            })
            .collect();
        line.tax = amounts.iter().sum();
        let priced_taxable = line.taxable;
        line.taxable = taxes.net_amount(&amounts);
        line.total = line.taxable + line.tax;
        if tax_inclusive && let Some(discount_tax) = line.net(priced_taxable) {
            applied.discount_tax += discount_tax;
            applied.netted[index] = true;
        }
    }
    applied
}

/// A document's header figures as its lines and the moves of its promotional credit left them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax: i64,
    pub promotional: i64,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub lines: Vec<Line>,
    /// The coupons' discount, spread over the lines' taxable amounts.
    pub discount: i64,
    pub tax_inclusive: bool,
    /// The subscription of a subscription's document; None for a consolidated one.
    pub subscription: Option<SubscriptionId>,
    pub header: Header,
}

impl Document {
    /// The totals its lines fold to (R-076).
    pub fn totals(&self) -> Totals {
        Totals::of(self.lines.iter().map(Line::amounts), self.discount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    pub(crate) fn line(id: &str, amount: i64) -> Line {
        Line {
            id: id.to_owned(),
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

    fn vat(rate: Decimal) -> Vec<Rate> {
        vec![Rate {
            rate,
            name: "VAT".to_owned(),
        }]
    }

    #[test]
    fn a_rate_is_rounded_once_over_the_lines_it_taxes() {
        let mut lines = vec![line("a", 1512), line("b", 1017)];
        let rates = [vat(dec!(0.2)), vat(dec!(0.2))];
        let rates: Vec<&[Rate]> = rates.iter().map(Vec::as_slice).collect();
        apply_rates(&mut lines, &rates, false);
        assert_eq!((lines[0].tax, lines[1].tax), (303, 203));
        assert_eq!(lines[0].total, 1815);
        assert_eq!(lines[0].taxes[0].amount, 303);
    }

    #[test]
    fn an_inclusive_line_is_netted_and_keeps_its_quote() {
        let mut quoted = line("a", 1200);
        quoted.tax_rate = dec!(0.2);
        let mut lines = vec![quoted];
        let rates = vat(dec!(0.2));
        let applied = apply_rates(&mut lines, &[rates.as_slice()], true);
        assert_eq!(applied.netted, vec![true]);
        assert_eq!(lines[0].figures.subtotal, 1000);
        assert_eq!(lines[0].quote.as_ref().map(|q| q.subtotal), Some(1200));
        assert_eq!(lines[0].taxable + lines[0].tax, 1200);
    }

    #[test]
    fn a_line_without_its_quote_is_grossed_up_by_its_rate() {
        let mut stored = line("a", 832);
        stored.tax_rate = dec!(0.2);
        stored.figures.unit_price = Some(dec!(8.32));
        let quoted = stored.into_quote();
        assert_eq!(quoted.figures.subtotal, 998);
        assert_eq!(quoted.figures.unit_price, Some(dec!(9.984)));
        assert_eq!((quoted.taxable, quoted.total, quoted.tax), (998, 998, 0));
    }

    #[test]
    fn a_netted_share_reads_net_on_the_document() {
        // 120.00 quoted at 20%, 50.00 of promotional credit: 58.33 net + 11.67 tax.
        let mut taxed = line("a", 10_000);
        taxed.taxable = 5_833;
        taxed.tax = 1_167;
        taxed.total = 7_000;
        taxed.promotional = Some(5_000);
        taxed.quote = Some(Figures {
            subtotal: 12_000,
            unit_price: None,
            sub_lines: Vec::new(),
        });
        assert_eq!(taxed.net_promotional(), 4_167);
    }
}
