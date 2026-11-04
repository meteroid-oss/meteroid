use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::ops::Mul;

/// Noise-stripping quantity scale: keeps any realistic per-unit price sub-cent accurate
/// while dropping metered aggregation's float-sum noise. Used for *step* pricing
/// (package/volume), where a coarser scale would round genuine fractional usage across a
/// block/tier boundary.
pub const MAX_QUANTITY_SCALE: u32 = 8;

/// Decimal places to round a billable quantity to, so one unit of the least-significant
/// digit is worth at most one minor currency unit (cheap units round toward whole, pricier
/// units keep more precision), also stripping aggregation float noise. Rates `<= 0` fall
/// back to [`MAX_QUANTITY_SCALE`].
///
/// Only sound for pricing *linear* in quantity (per-unit, graduated tiers, matrix cells);
/// step pricing (package/volume) must use [`MAX_QUANTITY_SCALE`]. Rates above
/// `10^(MAX_QUANTITY_SCALE - precision)` per unit are clamped and lose the one-unit bound.
pub fn quantity_scale(rate: Decimal, precision: u8) -> u32 {
    let magnitude = match rate.to_f64() {
        // `- 1e-9` so an exact power of ten resolves to `k`, not `k + 1`, despite float fuzz.
        Some(r) if r > 0.0 => (r.log10() - 1e-9).ceil() as i32,
        _ => return MAX_QUANTITY_SCALE,
    };
    (i32::from(precision) + magnitude).clamp(0, MAX_QUANTITY_SCALE as i32) as u32
}

/// Round a billable quantity to `scale` decimals half-up, normalized so a whole quantity
/// reads `100`, not `100.0`.
pub fn round_quantity(quantity: Decimal, scale: u32) -> Decimal {
    quantity
        .round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .normalize()
}

/// Round a usage quantity to its price-appropriate [`quantity_scale`]. Linear pricing only;
/// step pricing must round via [`round_quantity`] with [`MAX_QUANTITY_SCALE`].
pub fn round_billable_quantity(quantity: Decimal, rate: Decimal, precision: u8) -> Decimal {
    round_quantity(quantity, quantity_scale(rate, precision))
}

pub trait ToSubunit {
    fn to_subunit_opt(&self, precision: u8) -> Option<i64>;
}

impl ToSubunit for Decimal {
    fn to_subunit_opt(&self, precision: u8) -> Option<i64> {
        self.mul(Decimal::from(10i64.pow(u32::from(precision))))
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
    }
}

pub trait ToUnit {
    fn to_unit(&self, precision: u8) -> Decimal;
}

impl ToUnit for i64 {
    fn to_unit(&self, precision: u8) -> Decimal {
        Decimal::from(*self) / Decimal::from(10i64.pow(u32::from(precision)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn scale_follows_price_for_two_decimal_currency() {
        assert_eq!(quantity_scale(dec!(0.0001), 2), 0);
        assert_eq!(quantity_scale(dec!(0.001), 2), 0);
        assert_eq!(quantity_scale(dec!(0.02), 2), 1);
        assert_eq!(quantity_scale(dec!(0.10), 2), 1);
        assert_eq!(quantity_scale(dec!(1), 2), 2);
        assert_eq!(quantity_scale(dec!(10), 2), 3);
        assert_eq!(quantity_scale(dec!(100), 2), 4);
    }

    #[test]
    fn scale_respects_currency_precision() {
        assert_eq!(quantity_scale(dec!(0.02), 0), 0);
        assert_eq!(quantity_scale(dec!(1), 0), 0);
        assert_eq!(quantity_scale(dec!(100), 0), 2);
    }

    #[test]
    fn scale_is_bounded_and_handles_free_units() {
        assert_eq!(quantity_scale(dec!(0), 2), MAX_QUANTITY_SCALE);
        assert_eq!(quantity_scale(dec!(-5), 2), MAX_QUANTITY_SCALE);
        assert_eq!(quantity_scale(dec!(100000000), 2), MAX_QUANTITY_SCALE);
    }

    #[test]
    fn round_billable_quantity_drops_aggregation_noise() {
        assert_eq!(
            round_billable_quantity(dec!(5524.098500000001), dec!(0.02), 2),
            dec!(5524.1)
        );
        // No trailing-zero scale on whole quantities.
        let whole = round_billable_quantity(dec!(5524), dec!(0.02), 2);
        assert_eq!(whole, dec!(5524));
        assert_eq!(whole.to_string(), "5524");
        assert_eq!(
            round_billable_quantity(dec!(5524.098500000001), dec!(0.0001), 2),
            dec!(5524)
        );
    }

    #[test]
    fn per_unit_noise_rounding_keeps_the_billed_total() {
        // Not a universal guarantee: a raw→rounded shift can straddle a half-cent boundary
        // and move the total by one minor unit; here it does not.
        let raw = dec!(5524.098500000001);
        let rate = dec!(0.02);
        let rounded = round_billable_quantity(raw, rate, 2);
        let cents = |q: Decimal| (q * rate).round_dp(2);
        assert_eq!(cents(raw), cents(rounded));
    }

    #[test]
    fn noise_strip_scale_preserves_genuine_fractional_usage() {
        // At MAX_QUANTITY_SCALE float noise is dropped but genuine fractional usage survives.
        assert_eq!(
            round_quantity(dec!(1000.000000000058), MAX_QUANTITY_SCALE),
            dec!(1000)
        );
        assert_eq!(
            round_quantity(dec!(1000.4), MAX_QUANTITY_SCALE),
            dec!(1000.4)
        );
        assert_eq!(round_quantity(dec!(0.3), MAX_QUANTITY_SCALE), dec!(0.3));
    }
}
