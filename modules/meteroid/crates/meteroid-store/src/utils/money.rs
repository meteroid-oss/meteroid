use meteroid_money::FormattableCurrency;

/// `€800` for whole amounts, `€72,50` otherwise; unknown currencies fall back to the ISO code.
pub fn format_money(amount_minor: i64, currency: &str) -> String {
    let Some(cur) = meteroid_money::iso::find(currency) else {
        return format!("{amount_minor} {currency}");
    };
    let formatted = meteroid_money::Money::from_minor(amount_minor, cur).to_string();
    let exponent = cur.exponent();
    if exponent == 0 || amount_minor % 10_i64.pow(exponent) != 0 {
        return formatted;
    }
    let zeros = "0".repeat(exponent as usize);
    [".", ","]
        .iter()
        .find_map(|sep| formatted.strip_suffix(&format!("{sep}{zeros}")))
        .map(str::to_string)
        .unwrap_or(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_formatting() {
        assert_eq!(format_money(80000, "EUR"), "€800");
        assert_eq!(format_money(7250, "EUR"), "€72,50");
        assert_eq!(format_money(540000, "USD"), "$5,400");
        assert_eq!(format_money(1500, "JPY"), "¥1,500");
    }
}
