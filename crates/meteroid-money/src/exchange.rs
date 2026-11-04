use crate::currency::FormattableCurrency;
use crate::{Money, MoneyError};
use rust_decimal::Decimal;
use std::collections::HashMap;

/// Stores `ExchangeRate`s for easier access.
#[derive(Debug, Default)]
pub struct Exchange<'a, T: FormattableCurrency> {
    map: HashMap<(&'static str, &'static str), ExchangeRate<'a, T>>,
}

impl<'a, T: FormattableCurrency> Exchange<'a, T> {
    pub fn new() -> Exchange<'a, T> {
        Exchange {
            map: HashMap::new(),
        }
    }

    /// Update an ExchangeRate or add it if does not exist.
    pub fn set_rate(&mut self, rate: &ExchangeRate<'a, T>) {
        let key = Exchange::generate_key(rate.from, rate.to);
        self.map.insert(key, *rate);
    }

    /// Return the ExchangeRate given the currency pair.
    pub fn get_rate(&self, from: &T, to: &T) -> Option<ExchangeRate<'a, T>> {
        let key = Exchange::generate_key(from, to);
        self.map.get(&key).copied()
    }

    #[inline]
    fn generate_key(from: &T, to: &T) -> (&'static str, &'static str) {
        (from.code(), to.code())
    }
}

/// Stores rates of conversion between two currencies.
#[derive(Debug, PartialEq, Copy, Clone)]
pub struct ExchangeRate<'a, T: FormattableCurrency> {
    pub from: &'a T,
    pub to: &'a T,
    rate: Decimal,
}

impl<'a, T: FormattableCurrency> ExchangeRate<'a, T> {
    pub fn new(from: &'a T, to: &'a T, rate: Decimal) -> Result<ExchangeRate<'a, T>, MoneyError> {
        if from == to {
            return Err(MoneyError::InvalidCurrency);
        }
        Ok(ExchangeRate { from, to, rate })
    }

    /// Converts a Money from one Currency to another using the exchange rate.
    pub fn convert(&self, amount: &Money<'a, T>) -> Result<Money<'a, T>, MoneyError> {
        if amount.currency() != self.from {
            return Err(MoneyError::InvalidCurrency);
        }
        let converted_amount = amount.amount() * self.rate;
        Ok(Money::from_decimal(converted_amount, self.to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::define_currency_set;
    use rust_decimal_macros::*;

    define_currency_set!(
        test {
            USD: {
                code: "USD",
                exponent: 2,
                locale: EnUs,
                minor_units: 100,
                name: "USD",
                symbol: "$",
                symbol_first: true,
            },
            GBP : {
                code: "GBP",
                exponent: 2,
                locale: EnUs,
                minor_units: 1,
                name: "British Pound",
                symbol: "£",
                symbol_first: true,
            },
            EUR : {
                code: "EUR",
                exponent: 2,
                locale: EnEu,
                minor_units: 1,
                name: "Euro",
                symbol: "€",
                symbol_first: true,
            }
        }
    );

    #[test]
    fn exchange_stores_rates() {
        let usd = test::find("USD").unwrap();
        let eur = test::find("EUR").unwrap();
        let gbp = test::find("GBP").unwrap();

        let eur_usd_rate = ExchangeRate::new(usd, eur, dec!(1.5)).unwrap();
        let eur_gbp_rate = ExchangeRate::new(usd, gbp, dec!(1.6)).unwrap();

        let mut exchange = Exchange::new();
        exchange.set_rate(&eur_usd_rate);
        exchange.set_rate(&eur_gbp_rate);

        let fetched_rate = exchange.get_rate(usd, eur).unwrap();
        assert_eq!(fetched_rate.rate, dec!(1.5));

        let fetched_rate = exchange.get_rate(usd, gbp).unwrap();
        assert_eq!(fetched_rate.rate, dec!(1.6));
    }

    #[test]
    fn rate_convert() {
        let rate = ExchangeRate::new(test::USD, test::EUR, dec!(1.5)).unwrap();
        let amount = Money::from_minor(1_000, test::USD);
        let expected_amount = Money::from_minor(1_500, test::EUR);
        let converted_rate = rate.convert(&amount).unwrap();
        assert_eq!(converted_rate, expected_amount);
    }

    #[test]
    fn rate_convert_errors_if_currencies_do_not_match() {
        let rate = ExchangeRate::new(test::GBP, test::EUR, dec!(1.5)).unwrap();
        let amount = Money::from_minor(1_000, test::USD);

        assert_eq!(
            rate.convert(&amount).unwrap_err(),
            MoneyError::InvalidCurrency,
        );
    }

    #[test]
    fn rate_new_errors_if_currencies_are_equal() {
        let rate = ExchangeRate::new(test::GBP, test::GBP, dec!(1.5));
        assert_eq!(rate.unwrap_err(), MoneyError::InvalidCurrency,);
    }

    #[test]
    fn rate_with_zero_converts_to_zero() {
        // A zero exchange rate is mathematically valid (though unusual)
        let rate = ExchangeRate::new(test::USD, test::EUR, dec!(0)).unwrap();
        let amount = Money::from_minor(1000, test::USD);
        let converted = rate.convert(&amount).unwrap();
        assert_eq!(converted, Money::from_minor(0, test::EUR));
    }

    #[test]
    fn rate_with_negative_converts_correctly() {
        // Negative rates are unusual but mathematically valid
        let rate = ExchangeRate::new(test::USD, test::EUR, dec!(-1.5)).unwrap();
        let amount = Money::from_minor(1000, test::USD);
        let converted = rate.convert(&amount).unwrap();
        assert_eq!(converted, Money::from_minor(-1500, test::EUR));
    }

    #[test]
    fn rate_update_overwrites_existing() {
        let mut exchange = Exchange::new();

        let rate1 = ExchangeRate::new(test::USD, test::EUR, dec!(1.5)).unwrap();
        exchange.set_rate(&rate1);

        let rate2 = ExchangeRate::new(test::USD, test::EUR, dec!(2.0)).unwrap();
        exchange.set_rate(&rate2);

        let fetched = exchange.get_rate(test::USD, test::EUR).unwrap();
        assert_eq!(fetched.rate, dec!(2.0));
    }

    #[test]
    fn get_rate_returns_none_for_missing() {
        let exchange = Exchange::<test::Currency>::new();
        let result = exchange.get_rate(test::USD, test::EUR);
        assert!(result.is_none());
    }

    #[test]
    fn convert_zero_amount() {
        let rate = ExchangeRate::new(test::USD, test::EUR, dec!(1.5)).unwrap();
        let amount = Money::from_minor(0, test::USD);
        let converted = rate.convert(&amount).unwrap();
        assert!(converted.is_zero());
        assert_eq!(converted.currency(), test::EUR);
    }

    #[test]
    fn convert_preserves_precision() {
        // Test that small rates don't lose precision
        let rate = ExchangeRate::new(test::USD, test::EUR, dec!(0.000001)).unwrap();
        let amount = Money::from_minor(100_000_000, test::USD); // $1,000,000
        let converted = rate.convert(&amount).unwrap();
        // 1,000,000 * 0.000001 = 1.00
        assert_eq!(converted, Money::from_minor(100, test::EUR));
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use crate::define_currency_set;
    use proptest::prelude::*;
    use rust_decimal::Decimal;

    define_currency_set!(
        test {
            USD: {
                code: "USD",
                exponent: 2,
                locale: EnUs,
                minor_units: 100,
                name: "USD",
                symbol: "$",
                symbol_first: true,
            },
            EUR: {
                code: "EUR",
                exponent: 2,
                locale: EnEu,
                minor_units: 1,
                name: "Euro",
                symbol: "€",
                symbol_first: true,
            },
            GBP: {
                code: "GBP",
                exponent: 2,
                locale: EnUs,
                minor_units: 1,
                name: "British Pound",
                symbol: "£",
                symbol_first: true,
            }
        }
    );

    // Strategy for generating positive exchange rates
    fn positive_rate() -> impl Strategy<Value = Decimal> {
        (1i64..1_000_000i64, 0u32..6u32).prop_map(|(mantissa, scale)| Decimal::new(mantissa, scale))
    }

    // Strategy for generating Money amounts
    fn minor_amount() -> impl Strategy<Value = i64> {
        -1_000_000_000i64..1_000_000_000i64
    }

    proptest! {
        #[test]
        fn conversion_preserves_sign(amount in minor_amount(), rate in positive_rate()) {
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, rate).unwrap();
            let money = Money::from_minor(amount, test::USD);
            let converted = exchange_rate.convert(&money).unwrap();

            if money.is_positive() {
                prop_assert!(converted.is_positive(), "positive should stay positive");
            } else if money.is_negative() {
                prop_assert!(converted.is_negative(), "negative should stay negative");
            } else {
                prop_assert!(converted.is_zero(), "zero should stay zero");
            }
        }

        #[test]
        fn conversion_of_zero_is_zero(rate in positive_rate()) {
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, rate).unwrap();
            let zero = Money::from_minor(0, test::USD);
            let converted = exchange_rate.convert(&zero).unwrap();

            prop_assert!(converted.is_zero());
        }

        #[test]
        fn converted_currency_is_target(amount in minor_amount(), rate in positive_rate()) {
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, rate).unwrap();
            let money = Money::from_minor(amount, test::USD);
            let converted = exchange_rate.convert(&money).unwrap();

            prop_assert_eq!(converted.currency(), test::EUR);
        }

        #[test]
        fn rate_of_one_preserves_amount(amount in minor_amount()) {
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, Decimal::ONE).unwrap();
            let money = Money::from_minor(amount, test::USD);
            let converted = exchange_rate.convert(&money).unwrap();

            // Amount should be the same, just different currency
            prop_assert_eq!(money.amount(), converted.amount());
        }

        #[test]
        fn exchange_set_get_roundtrip(rate in positive_rate()) {
            let mut exchange = Exchange::new();
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, rate).unwrap();

            exchange.set_rate(&exchange_rate);
            let retrieved = exchange.get_rate(test::USD, test::EUR);

            prop_assert!(retrieved.is_some());
            prop_assert_eq!(retrieved.unwrap(), exchange_rate);
        }

        #[test]
        fn exchange_missing_rate_returns_none(rate in positive_rate()) {
            let mut exchange = Exchange::new();
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, rate).unwrap();

            exchange.set_rate(&exchange_rate);

            // Different currency pair should return None
            let retrieved = exchange.get_rate(test::USD, test::GBP);
            prop_assert!(retrieved.is_none());

            // Reversed direction should also return None
            let reversed = exchange.get_rate(test::EUR, test::USD);
            prop_assert!(reversed.is_none());
        }

        #[test]
        fn conversion_scales_linearly(amount in 1i64..1_000_000, factor in 1i64..100) {
            let rate = Decimal::new(15, 1); // 1.5
            let exchange_rate = ExchangeRate::new(test::USD, test::EUR, rate).unwrap();

            let money1 = Money::from_minor(amount, test::USD);
            let money2 = Money::from_minor(amount * factor, test::USD);

            let converted1 = exchange_rate.convert(&money1).unwrap();
            let converted2 = exchange_rate.convert(&money2).unwrap();

            // converted2 should be factor times converted1
            let scaled = *converted1.amount() * Decimal::from(factor);
            prop_assert_eq!(scaled, *converted2.amount());
        }
    }
}
