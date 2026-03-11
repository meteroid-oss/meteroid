use tax_ids::TaxId;

pub fn validate_vat_number_format(vat_number: &str) -> bool {
    if vat_number.len() < 2 {
        return false;
    }
    // TODO   assert_eq!(tax_id.country_code(), customer_country_code);
    TaxId::new(vat_number).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_strings_do_not_panic() {
        assert!(!validate_vat_number_format(""));
        assert!(!validate_vat_number_format("0"));
        assert!(!validate_vat_number_format("1"));
        assert!(!validate_vat_number_format("X"));
        assert!(!validate_vat_number_format("22"));
    }

    #[test]
    fn test_valid_vat_number() {
        assert!(validate_vat_number_format("FR12345678901"));
    }

    #[test]
    fn test_invalid_vat_number_format() {
        assert!(!validate_vat_number_format("XX00000000000"));
    }
}
