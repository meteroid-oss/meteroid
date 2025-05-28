use rust_decimal::Decimal;

pub fn only_positive(price_cents: i64) -> u64 {
    if price_cents > 0 {
        price_cents as u64
    } else {
        0
    }
}

pub fn only_positive_decimal(price: Decimal) -> Decimal {
    if price.is_sign_positive() {
        price
    } else {
        Decimal::from(0)
    }
}
