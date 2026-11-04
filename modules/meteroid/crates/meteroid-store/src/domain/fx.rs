//! Minor-unit conversion into the reporting currency.

use std::collections::{BTreeMap, HashMap};

use crate::constants::Currencies;

/// Minor-unit conversion factors into one target currency, shared by Rust and SQL.
#[derive(Clone, Debug, PartialEq)]
pub struct FxTable {
    target: String,
    factors: HashMap<String, f64>,
}

impl FxTable {
    /// Converts only the target currency (no rates available).
    pub fn identity(target: &str) -> Self {
        FxTable {
            target: target.to_string(),
            factors: HashMap::from([(target.to_string(), 1.0)]),
        }
    }

    /// `rates_from_usd[c]` is the amount of `c` one USD buys (the `historical_rates_from_usd` row).
    pub fn from_usd_rates(target: &str, rates_from_usd: &BTreeMap<String, f64>) -> Self {
        let mut table = Self::identity(target);
        let usd_rate = |code: &str| match rates_from_usd.get(code) {
            Some(r) if r.is_finite() && *r > 0.0 => Some(*r),
            _ if code == "USD" => Some(1.0),
            _ => None,
        };
        let (Some(target_rate), Some(target_prec)) = (
            usd_rate(target),
            Currencies::resolve_currency_precision(target),
        ) else {
            return table;
        };
        let codes = rates_from_usd.keys().map(String::as_str).chain(["USD"]);
        for code in codes {
            if code == target {
                continue;
            }
            let (Some(rate), Some(prec)) =
                (usd_rate(code), Currencies::resolve_currency_precision(code))
            else {
                continue;
            };
            let scale = 10_f64.powi(i32::from(target_prec) - i32::from(prec));
            table
                .factors
                .insert(code.to_string(), target_rate / rate * scale);
        }
        table
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn factor(&self, currency: &str) -> Option<f64> {
        self.factors.get(currency).copied()
    }

    /// Rounds half away from zero; `None` for a currency without a rate.
    pub fn convert(&self, amount_minor: i64, currency: &str) -> Option<i64> {
        if currency == self.target {
            return Some(amount_minor);
        }
        let converted = (amount_minor as f64 * self.factor(currency)?).round();
        converted.is_finite().then_some(converted as i64)
    }

    /// Sum of per-currency amounts; currencies without a rate are left out.
    pub fn convert_sum<'a>(&self, amounts: impl IntoIterator<Item = (&'a str, i64)>) -> i64 {
        amounts
            .into_iter()
            .filter_map(|(currency, amount)| self.convert(amount, currency))
            .sum()
    }

    /// `(codes, factors)` for `unnest($codes::text[], $factors::float8[])`, sorted by code.
    pub fn as_arrays(&self) -> (Vec<String>, Vec<f64>) {
        let mut pairs: Vec<_> = self.factors.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        pairs.into_iter().map(|(c, f)| (c.clone(), *f)).unzip()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fx_table_converts_minor_units_across_precisions() {
        let rates = BTreeMap::from([
            ("EUR".to_string(), 0.8),
            ("JPY".to_string(), 150.0),
            ("BHD".to_string(), 0.376),
        ]);
        let eur = FxTable::from_usd_rates("EUR", &rates);
        assert_eq!(eur.target(), "EUR");
        assert_eq!(eur.factor("EUR"), Some(1.0));
        assert_eq!(eur.convert(123_456_789, "EUR"), Some(123_456_789));
        assert_eq!(eur.convert(10_000, "USD"), Some(8_000));
        assert_eq!(eur.convert(15_000, "JPY"), Some(8_000));
        assert_eq!(eur.convert(-15_000, "JPY"), Some(-8_000));
        assert_eq!(eur.convert(100, "XYZ"), None);
        assert_eq!(
            eur.convert_sum([("EUR", 100), ("USD", 100), ("XYZ", 5)]),
            180
        );

        let jpy = FxTable::from_usd_rates("JPY", &rates);
        assert_eq!(jpy.convert(8_000, "EUR"), Some(15_000));
        assert_eq!(jpy.convert(1, "USD"), Some(2));
        assert_eq!(jpy.convert(376, "BHD"), Some(150));

        let (codes, factors) = eur.as_arrays();
        assert_eq!(codes, vec!["BHD", "EUR", "JPY", "USD"]);
        assert_eq!(factors[1], 1.0);

        let bare = FxTable::from_usd_rates("CHF", &rates);
        assert_eq!(bare.as_arrays(), (vec!["CHF".to_string()], vec![1.0]));
        assert_eq!(FxTable::identity("EUR").convert(5, "USD"), None);
    }
}
