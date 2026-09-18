use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

/// `value` is an exact decimal string (`"10.00"`) with the currency's ISO 4217 exponent as scale.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Amount {
    pub currency: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub value: Decimal,
}

impl Amount {
    /// Formats with exactly `exponent` decimals (`0` for JPY/ISK).
    pub fn from_minor(minor: i64, currency: &str, exponent: u32) -> Self {
        Amount {
            currency: currency.to_ascii_uppercase(),
            value: Decimal::from_i128_with_scale(minor as i128, exponent),
        }
    }

    /// Errors on too many decimals or `i64` overflow; never truncates.
    pub fn to_minor(&self, exponent: u32) -> Result<i64, String> {
        let scaled = self
            .value
            .checked_mul(Decimal::from(10_i64.pow(exponent)))
            .ok_or_else(|| format!("mollie amount overflow: {}", self.value))?;
        if scaled.fract() != Decimal::ZERO {
            return Err(format!(
                "invalid mollie amount {} {} for exponent {exponent}",
                self.currency, self.value
            ));
        }
        scaled
            .to_i64()
            .ok_or_else(|| format!("mollie amount too large: {}", self.value))
    }
}

#[cfg(test)]
mod tests {
    use super::Amount;
    use rust_decimal_macros::dec;

    fn amount(currency: &str, value: rust_decimal::Decimal) -> Amount {
        Amount {
            currency: currency.into(),
            value,
        }
    }

    #[test]
    fn serializes_with_the_currency_scale() {
        let json = |a: Amount| serde_json::to_value(a).unwrap()["value"].clone();
        assert_eq!(json(Amount::from_minor(1000, "eur", 2)), "10.00");
        assert_eq!(Amount::from_minor(1000, "eur", 2).currency, "EUR");
        assert_eq!(json(Amount::from_minor(5, "USD", 2)), "0.05");
        assert_eq!(json(Amount::from_minor(0, "GBP", 2)), "0.00");
        assert_eq!(json(Amount::from_minor(-1250, "EUR", 2)), "-12.50");
        assert_eq!(json(Amount::from_minor(1500, "JPY", 0)), "1500");
        assert_eq!(json(Amount::from_minor(7, "ISK", 0)), "7");
        assert_eq!(json(Amount::from_minor(12345, "KWD", 3)), "12.345");
    }

    #[test]
    fn deserializes_from_a_json_string_only() {
        let parsed: Amount = serde_json::from_str(r#"{"currency":"EUR","value":"24.95"}"#).unwrap();
        assert_eq!(parsed.value, dec!(24.95));
        assert!(serde_json::from_str::<Amount>(r#"{"currency":"EUR","value":24.95}"#).is_err());
        assert!(serde_json::from_str::<Amount>(r#"{"currency":"EUR","value":"abc"}"#).is_err());
    }

    #[test]
    fn parses_back_to_minor() {
        assert_eq!(amount("EUR", dec!(10.00)).to_minor(2), Ok(1000));
        assert_eq!(amount("EUR", dec!(10.5)).to_minor(2), Ok(1050));
        assert_eq!(amount("JPY", dec!(1500)).to_minor(0), Ok(1500));
        assert_eq!(amount("EUR", dec!(-35.07)).to_minor(2), Ok(-3507));
        // More decimals than the currency allows: error, not truncation.
        assert!(amount("EUR", dec!(10.005)).to_minor(2).is_err());
        assert!(amount("EUR", dec!(10.50)).to_minor(0).is_err());
        assert_eq!(amount("JPY", dec!(1500)).to_minor(2), Ok(150000));
    }

    #[test]
    fn round_trips() {
        for (minor, cur, exp) in [
            (1, "EUR", 2),
            (99, "USD", 2),
            (123456789, "CHF", 2),
            (42, "JPY", 0),
        ] {
            assert_eq!(Amount::from_minor(minor, cur, exp).to_minor(exp), Ok(minor));
        }
    }
}
