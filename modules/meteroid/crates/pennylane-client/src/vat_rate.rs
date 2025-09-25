use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[allow(non_camel_case_types)]
pub enum VatRate {
    FR_200,
    FR_196,
    FR_160,
    FR_130,
    FR_100,
    FR_92,
    FR_85,
    FR_65,
    FR_60,
    FR_55,
    FR_50,
    FR_40,
    FR_21,
    FR_15_385,
    FR_09,
    FR_1_75,
    FR_1_05,
    #[serde(rename = "exempt")]
    EXEMPT,
}

impl VatRate {
    pub fn from_decimal(rate: Decimal, country_name: &str) -> Option<Self> {
        let country = country_name.to_lowercase();
        match country.as_str() {
            "france" | "fr" => match rate {
                r if r == dec!(0.2) => Some(VatRate::FR_200),
                r if r == dec!(0.196) => Some(VatRate::FR_196),
                r if r == dec!(0.16) => Some(VatRate::FR_160),
                r if r == dec!(0.13) => Some(VatRate::FR_130),
                r if r == dec!(0.1) => Some(VatRate::FR_100),
                r if r == dec!(0.092) => Some(VatRate::FR_92),
                r if r == dec!(0.085) => Some(VatRate::FR_85),
                r if r == dec!(0.065) => Some(VatRate::FR_65),
                r if r == dec!(0.06) => Some(VatRate::FR_60),
                r if r == dec!(0.055) => Some(VatRate::FR_55),
                r if r == dec!(0.05) => Some(VatRate::FR_50),
                r if r == dec!(0.04) => Some(VatRate::FR_40),
                r if r == dec!(0.021) => Some(VatRate::FR_21),
                r if r == dec!(0.15385) => Some(VatRate::FR_15_385),
                r if r == dec!(0.009) => Some(VatRate::FR_09),
                r if r == dec!(0.0175) => Some(VatRate::FR_1_75),
                r if r == dec!(0.0105) => Some(VatRate::FR_1_05),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_decimal() {
        // Test all French tax rates
        assert_eq!(
            VatRate::from_decimal(dec!(0.2), "FR"),
            Some(VatRate::FR_200)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.196), "FR"),
            Some(VatRate::FR_196)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.16), "FR"),
            Some(VatRate::FR_160)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.13), "FR"),
            Some(VatRate::FR_130)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.1), "FR"),
            Some(VatRate::FR_100)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.092), "FR"),
            Some(VatRate::FR_92)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.085), "FR"),
            Some(VatRate::FR_85)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.065), "FR"),
            Some(VatRate::FR_65)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.06), "FR"),
            Some(VatRate::FR_60)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.055), "FR"),
            Some(VatRate::FR_55)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.05), "FR"),
            Some(VatRate::FR_50)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.04), "FR"),
            Some(VatRate::FR_40)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.021), "FR"),
            Some(VatRate::FR_21)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.15385), "FR"),
            Some(VatRate::FR_15_385)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.009), "FR"),
            Some(VatRate::FR_09)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.0175), "FR"),
            Some(VatRate::FR_1_75)
        );
        assert_eq!(
            VatRate::from_decimal(dec!(0.0105), "FR"),
            Some(VatRate::FR_1_05)
        );

        // Test unmatched rates and countries
        assert_eq!(VatRate::from_decimal(dec!(0.999), "FR"), None);
        assert_eq!(VatRate::from_decimal(dec!(0.2), "US"), None);
    }
}
