use crate::errors::{InvoicingError, InvoicingResult};
use chrono::NaiveDate;
use common_domain::l10n::chrono_locale;
pub(crate) use common_domain::l10n::long_date;
use meteroid_money::{FormattableCurrency, iso};
use rust_decimal::Decimal;
use std::str::FromStr;
use typst::foundations::Decimal as TypstDecimal;

pub(crate) type Price = meteroid_money::Money<'static, iso::Currency>;

/// Usage-based unit prices are routinely sub-cent, so they keep more precision than totals
/// instead of being rounded to the currency exponent. Must match `formatCurrencyNoRounding`
/// in `web-app/lib/utils/numbers.ts`, so the PDF and the invoice page show the same price.
const MAX_UNIT_PRICE_SCALE: u32 = 12;

/// Renders a monetary amount at the currency exponent (e.g. `1234.50` for EUR).
/// Typst decimals print their scale verbatim, so the padding is decided here.
pub(crate) fn money(money: &Price) -> InvoicingResult<TypstDecimal> {
    rescaled(*money.amount(), money.currency().exponent())
}

/// Renders a unit price, keeping the significant decimals beyond the currency exponent
/// so sub-cent prices are not displayed as a rounded-up minimum (`0.005` stays `0.005`).
pub(crate) fn unit_price(money: &Price) -> InvoicingResult<TypstDecimal> {
    let amount = money.amount().round_dp(MAX_UNIT_PRICE_SCALE).normalize();
    rescaled(amount, amount.scale().max(money.currency().exponent()))
}

/// A caller-supplied strftime pattern (annex date presets) rendered in `lang`.
pub(crate) fn pattern_date(lang: &str, date: &NaiveDate, fmt: &str) -> String {
    match chrono_locale(lang) {
        Some(locale) => date.format_localized(fmt, locale).to_string(),
        None => date.format(fmt).to_string(),
    }
}

/// A count written out in `lang`'s digit grouping and decimal mark: `1,000.5` in English is
/// `1 000,5` in French, where `1,000` would read as one.
pub(crate) fn grouped_number(lang: &str, value: Decimal) -> String {
    let (group, decimal) = match lang {
        "en-US" => (",", "."),
        "fr-FR" | "pt-PT" => ("\u{00A0}", ","),
        _ => (".", ","),
    };
    let text = value.normalize().to_string();
    let (sign, unsigned) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |rest| ("-", rest));
    let (whole, fraction) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(whole, fraction)| {
            (whole, Some(fraction))
        });
    let mut grouped = String::with_capacity(whole.len() * 2);
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push_str(group);
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}{decimal}{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

fn rescaled(amount: Decimal, scale: u32) -> InvoicingResult<TypstDecimal> {
    let mut amount = amount.round_dp(scale);
    amount.rescale(scale);
    TypstDecimal::from_str(&amount.to_string()).map_err(|err| {
        InvoicingError::InvoiceGenerationError(format!("amount {amount} is not renderable: {err}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::prelude::FromPrimitive;

    fn eur(amount: &str) -> Price {
        Price::from_decimal(Decimal::from_str(amount).unwrap(), iso::EUR)
    }

    #[test]
    fn counts_are_grouped_in_the_documents_language() {
        let value = Decimal::from_str("1234567.5").unwrap();
        assert_eq!(grouped_number("en-US", value), "1,234,567.5");
        assert_eq!(grouped_number("fr-FR", value), "1\u{00A0}234\u{00A0}567,5");
        assert_eq!(grouped_number("de-DE", value), "1.234.567,5");
        assert_eq!(grouped_number("pt-PT", value), "1\u{00A0}234\u{00A0}567,5");
        assert_eq!(grouped_number("en-US", Decimal::from(500)), "500");
        assert_eq!(
            grouped_number("fr-FR", Decimal::from_str("1000.00").unwrap()),
            "1\u{00A0}000"
        );
    }

    #[test]
    fn money_is_padded_to_the_currency_exponent() {
        assert_eq!(money(&eur("50")).unwrap().to_string(), "50.00");
        assert_eq!(money(&eur("0.04")).unwrap().to_string(), "0.04");
        assert_eq!(
            money(&Price::from_decimal(
                Decimal::from_f64(1234.0).unwrap(),
                iso::JPY
            ))
            .unwrap()
            .to_string(),
            "1234"
        );
    }

    #[test]
    fn unit_price_keeps_sub_cent_precision() {
        assert_eq!(unit_price(&eur("0.005")).unwrap().to_string(), "0.005");
        assert_eq!(
            unit_price(&eur("0.0000015")).unwrap().to_string(),
            "0.0000015"
        );
        assert_eq!(unit_price(&eur("0.005000")).unwrap().to_string(), "0.005");
    }

    #[test]
    fn unit_price_keeps_the_currency_exponent_as_a_minimum() {
        assert_eq!(unit_price(&eur("50")).unwrap().to_string(), "50.00");
        assert_eq!(unit_price(&eur("0.5")).unwrap().to_string(), "0.50");
    }

    #[test]
    fn unit_price_precision_is_capped() {
        assert_eq!(
            unit_price(&eur("0.1234567890123456")).unwrap().to_string(),
            "0.123456789012"
        );
    }
}
