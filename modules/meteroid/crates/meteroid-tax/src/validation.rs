use lazy_regex::{Regex, regex};

/// Returns the country-specific VAT number regex for the given 2-letter prefix.
///
/// Patterns vendored from https://github.com/Mollemoll/tax-ids (MIT/Apache-2.0),
/// covering EU VAT, GB, CH and NO. Only the syntax validation is inlined; the
/// upstream crate's network verification (VIES/HMRC/...) is handled separately.
fn vat_pattern(country_prefix: &str) -> Option<&'static Regex> {
    Some(match country_prefix {
        // EU
        "AT" => regex!(r"^ATU[0-9]{8}$"),
        "BE" => regex!(r"^BE[0-1][0-9]{9}$"),
        "BG" => regex!(r"^BG[0-9]{9,10}$"),
        "CY" => regex!(r"^CY[0-69][0-9]{7}[A-Z]$"),
        "CZ" => regex!(r"^CZ[0-9]{8,10}$"),
        "DE" => regex!(r"^DE[0-9]{9}$"),
        "DK" => regex!(r"^DK[0-9]{8}$"),
        "EE" => regex!(r"^EE10[0-9]{7}$"),
        "EL" => regex!(r"^EL[0-9]{9}$"),
        "ES" => regex!(r"^ES([A-Z][0-9]{8}|[0-9]{8}[A-Z]|[A-Z][0-9]{7}[A-Z])$"),
        "FI" => regex!(r"^FI[0-9]{8}$"),
        "FR" => regex!(r"^FR[A-HJ-NP-Z0-9]{2}[0-9]{9}$"),
        "HR" => regex!(r"^HR[0-9]{11}$"),
        "HU" => regex!(r"^HU[0-9]{8}$"),
        "IE" => regex!(r"^IE([0-9][A-Z][0-9]{5}|[0-9]{7}[A-Z]?)[A-Z]$"),
        "IT" => regex!(r"^IT[0-9]{11}$"),
        "LT" => regex!(r"^LT([0-9]{7}1[0-9]|[0-9]{10}1[0-9])$"),
        "LU" => regex!(r"^LU[0-9]{8}$"),
        "LV" => regex!(r"^LV[0-9]{11}$"),
        "MT" => regex!(r"^MT[0-9]{8}$"),
        "NL" => regex!(r"^NL[0-9]{9}B[0-9]{2}$"),
        "PL" => regex!(r"^PL[0-9]{10}$"),
        "PT" => regex!(r"^PT[0-9]{9}$"),
        "RO" => regex!(r"^RO[1-9][0-9]{1,9}$"),
        "SE" => regex!(r"^SE[0-9]{10}01$"),
        "SI" => regex!(r"^SI[0-9]{8}$"),
        "SK" => regex!(r"^SK[0-9]{10}$"),
        "XI" => regex!(r"^XI([0-9]{9}|[0-9]{12}|(HA|GD)[0-9]{3})$"),
        // Non-EU
        "GB" => regex!(r"^GB([0-9]{9}|[0-9]{12}|(HA|GD)[0-9]{3})$"),
        "CH" => regex!(r"^CHE([0-9]{9}|-[0-9]{3}(\.[0-9]{3}){2})(?:\s(MWST|TVA|IVA))?$"),
        "NO" => regex!(r"^NO[0-9]{9}(MVA)?$"),
        _ => return None,
    })
}

/// Validates a VAT number against the country-specific syntax derived from its
/// 2-letter prefix. This is a purely local, offline check; it does not confirm
/// the number is registered (see the VIES verification path for that).
pub fn validate_vat_number_format(vat_number: &str) -> bool {
    // Trim first, matching the VIES path (`vies::split_vat_number`), so numbers
    // with surrounding whitespace aren't spuriously rejected.
    let vat_number = vat_number.trim();
    match vat_number.get(0..2) {
        Some(prefix) => vat_pattern(prefix).is_some_and(|re| re.is_match(vat_number)),
        None => false,
    }
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

    #[test]
    fn test_non_eu_prefixes() {
        assert!(validate_vat_number_format("GB123456789"));
        assert!(validate_vat_number_format("NO123456789MVA"));
        assert!(validate_vat_number_format("CHE123456789"));
    }
}
