//! What a fee's quantity costs: the price models, fixed-fee lines and their proration, in minor
//! units with Decimal intermediates. The caller resolves prices, units and usage.

mod fees;
mod usage;

pub use fees::{tiered, usage_amount, usage_price, volume};
pub use usage::{capacity_overage_lines, cell_matches, usage_lines};

use std::collections::HashMap;

use common_utils::decimals::{MAX_QUANTITY_SCALE, ToSubunit, quantity_scale};
use common_utils::integers::only_positive;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::proration::DayFraction;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PricingError {
    /// An amount or quantity beyond what minor units in `i64` hold.
    OutOfRange,
    /// A credit pack granting a negative amount, or a fraction of its minor unit.
    InvalidPackCredit,
    /// A price no quantity can be billed under: a package of no units, or two tiers starting on
    /// the same unit.
    InvalidPrice,
}

/// One row of a tiered or volume price; its last unit is the next row's first.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier {
    pub first_unit: u64,
    pub rate: Decimal,
    pub flat_fee: Option<Decimal>,
    pub flat_cap: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatrixDimension {
    pub key: String,
    pub value: String,
}

/// A matrix price: the usage carrying `dimension1` (and `dimension2`) costs `per_unit_price`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixCell {
    pub dimension1: MatrixDimension,
    pub dimension2: Option<MatrixDimension>,
    pub per_unit_price: Decimal,
}

impl MatrixCell {
    /// Whether usage carrying `dimensions` falls in this cell.
    pub fn matches(&self, dimensions: &HashMap<String, String>) -> bool {
        cell_matches(
            (&self.dimension1.key, &self.dimension1.value),
            self.dimension2
                .as_ref()
                .map(|dimension2| (dimension2.key.as_str(), dimension2.value.as_str())),
            dimensions,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UsageModel {
    PerUnit {
        rate: Decimal,
    },
    Tiered {
        tiers: Vec<Tier>,
        block_size: Option<u64>,
    },
    Volume {
        tiers: Vec<Tier>,
        block_size: Option<u64>,
    },
    Package {
        block_size: u64,
        rate: Decimal,
    },
    Matrix {
        rates: Vec<MatrixCell>,
    },
}

impl UsageModel {
    /// Decimal places to round this model's billable quantity to, at currency `precision`.
    ///
    /// Linear models (per-unit, graduated tiers) use the price-derived `quantity_scale` of
    /// their largest rate; step models (package, volume) only strip float noise, since a
    /// coarser scale would round genuine fractional usage across a `ceil`/tier boundary and
    /// drop a whole block of revenue. Matrix is priced per cell at the call site.
    pub fn billable_quantity_scale(&self, precision: u8) -> u32 {
        match self {
            UsageModel::PerUnit { rate } => quantity_scale(*rate, precision),
            UsageModel::Tiered { tiers, .. } => tiers
                .iter()
                .map(|t| t.rate)
                .max()
                .map_or(MAX_QUANTITY_SCALE, |rate| quantity_scale(rate, precision)),
            UsageModel::Package { .. } | UsageModel::Volume { .. } | UsageModel::Matrix { .. } => {
                MAX_QUANTITY_SCALE
            }
        }
    }
}

/// Usage of one group of dimensions over a window.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupedUsage {
    pub value: Decimal,
    pub dimensions: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubLineAttributes {
    Package {
        raw_usage: Decimal,
    },
    Tiered {
        first_unit: u64,
        last_unit: Option<u64>,
        flat_cap: Option<Decimal>,
        flat_fee: Option<Decimal>,
    },
    Volume {
        first_unit: u64,
        last_unit: Option<u64>,
        flat_cap: Option<Decimal>,
        flat_fee: Option<Decimal>,
    },
    Matrix {
        dimension1_key: String,
        dimension1_value: String,
        dimension2_key: Option<String>,
        dimension2_value: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubLine {
    pub name: String,
    pub total: i64,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub attributes: Option<SubLineAttributes>,
}

/// A priced line, before the caller gives it its period, fee and metric.
#[derive(Clone, Debug, PartialEq)]
pub struct PricedLine {
    pub total: u64,
    pub quantity: Option<Decimal>,
    pub unit_price: Option<Decimal>,
    pub is_prorated: bool,
    pub sublines: Vec<SubLine>,
    pub dimensions: Option<HashMap<String, String>>,
}

impl PricedLine {
    /// A line made of sublines. Rules: R-071, R-002.
    pub fn from_usage_sublines(
        mut sublines: Vec<SubLine>,
        proration_factor: Option<DayFraction>,
    ) -> PricedLine {
        let total = sublines.iter().map(|subline| subline.total).sum::<i64>();
        let total_cents = prorate(total, proration_factor);

        // When prorating, apply the same factor to each subline so the invariant
        // `line.total == Σ sublines.total` holds. Any rounding delta is absorbed
        // by the largest-magnitude subline (largest-remainder correction).
        if proration_factor.is_some_and(|f| !f.is_whole()) && !sublines.is_empty() {
            for subline in sublines.iter_mut() {
                subline.total = prorate(subline.total, proration_factor) as i64;
            }
            let current_sum: i64 = sublines.iter().map(|s| s.total).sum();
            let diff = total_cents as i64 - current_sum;
            if diff != 0
                && let Some((idx, _)) = sublines
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, s)| s.total.unsigned_abs())
            {
                sublines[idx].total += diff;
            }
        }

        PricedLine {
            quantity: None,
            unit_price: None,
            total: total_cents,
            is_prorated: proration_factor.is_some_and(|f| !f.is_whole()),
            sublines,
            dimensions: None,
        }
    }
}

/// `amount` in minor units at `precision`, rounded half away from zero (R-001).
pub(crate) fn to_minor(amount: Decimal, precision: u8) -> Result<i64, PricingError> {
    amount
        .to_subunit_opt(precision)
        .ok_or(PricingError::OutOfRange)
}

/// `rate × quantity`, rounded to the minor unit, then prorated.
pub fn simple_prorated(
    rate: &Decimal,
    quantity: &Decimal,
    proration_factor: Option<DayFraction>,
    precision: u8,
) -> Result<PricedLine, PricingError> {
    let total = rate
        .checked_mul(*quantity)
        .ok_or(PricingError::OutOfRange)?;

    let total_cents = prorate(to_minor(total, precision)?, proration_factor);

    // When prorated, derive unit_price from the rounded total so that
    // unit_price × quantity = total exactly.
    // When not prorated, use the original rate directly to avoid precision
    // artifacts from rounding total_cents then dividing back by quantity.
    let unit_price = if proration_factor.is_some() {
        if quantity.is_zero() {
            Decimal::ZERO
        } else {
            let divisor = Decimal::from(10u64.pow(precision as u32));
            Decimal::from(total_cents as i64) / divisor / quantity
        }
    } else {
        *rate
    };

    Ok(PricedLine {
        quantity: Some(*quantity),
        unit_price: Some(unit_price),
        total: total_cents,
        is_prorated: proration_factor.is_some_and(|f| !f.is_whole()),
        sublines: Vec::new(),
        dimensions: None,
    })
}

/// A credit pack's charge: `units` packs at `rate`, a period-tied pack taking its `share` of a
/// partial first period, its unit price then read back from the rounded total.
pub fn credit_pack(
    rate: &Decimal,
    units: Decimal,
    share: Option<DayFraction>,
    precision: u8,
) -> Result<PricedLine, PricingError> {
    let full = to_minor(
        rate.checked_mul(units).ok_or(PricingError::OutOfRange)?,
        precision,
    )?;
    let total = share.map_or(full, |share| share.prorate(full));
    let unit_price = match share {
        Some(_) if !units.is_zero() => Decimal::new(total, u32::from(precision)) / units,
        _ => *rate,
    };
    Ok(PricedLine {
        total: u64::try_from(total.max(0)).map_err(|_| PricingError::OutOfRange)?,
        quantity: Some(units),
        unit_price: Some(unit_price),
        is_prorated: share.is_some_and(|share| !share.is_whole()),
        sublines: Vec::new(),
        dimensions: None,
    })
}

/// Credit a pack sells, in whole minor units of the invoice currency. A fractional minor unit is
/// refused rather than rounded: the grant must be exactly what was sold.
pub fn pack_credit_minor(
    credit_amount: Decimal,
    units: Decimal,
    precision: u8,
) -> Result<i64, PricingError> {
    credit_amount
        .checked_mul(units)
        .and_then(|amount| amount.checked_mul(Decimal::from(10_i64.pow(u32::from(precision)))))
        .filter(|amount| amount.fract().is_zero() && !amount.is_sign_negative())
        .and_then(|amount| amount.to_i64())
        .ok_or(PricingError::InvalidPackCredit)
}

/// The seats a slot fee bills: those held, within the fee's bounds.
pub fn billed_slots(held: u64, min_slots: Option<u32>, max_slots: Option<u32>) -> u64 {
    held.max(u64::from(min_slots.unwrap_or(0)))
        .min(u64::from(max_slots.unwrap_or(u32::MAX)))
}

/// Whole days over whole days, half-up once (OQ14): a true half-cent is never flipped.
pub(crate) fn prorate(price_cents: i64, proration_factor: Option<DayFraction>) -> u64 {
    only_positive(proration_factor.map_or(price_cents, |factor| factor.prorate(price_cents)))
}

/// How a metric's unit conversion rounds a period total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitRounding {
    Up,
    Down,
    Nearest,
    NearestHalf,
    NearestDecile,
    None,
}

/// A period total of `units` in the metric's billed unit.
pub fn converted_units(
    factor: Option<i32>,
    rounding: Option<UnitRounding>,
    units: Decimal,
) -> Decimal {
    // Rounding belongs to the conversion: without a factor the usage is billed as metered.
    if factor.is_none_or(|factor| factor == 0) {
        return units;
    }
    let converted = unrounded_units(factor, units);
    match rounding {
        None | Some(UnitRounding::None) => converted,
        Some(UnitRounding::Up) => converted.ceil(),
        Some(UnitRounding::Down) => converted.floor(),
        Some(UnitRounding::Nearest) => {
            converted.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        }
        Some(UnitRounding::NearestHalf) => {
            (converted * Decimal::TWO)
                .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
                / Decimal::TWO
        }
        Some(UnitRounding::NearestDecile) => {
            converted.round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero)
        }
    }
}

/// The conversion without the rounding, which applies to a period total only: for a share of it
/// (one event, one bucket), so the shares stay proportional to the usage they carry.
pub fn unrounded_units(factor: Option<i32>, units: Decimal) -> Decimal {
    match factor {
        Some(factor) if factor != 0 => units / Decimal::from(factor),
        _ => units,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn subline(name: &str, total: i64) -> SubLine {
        SubLine {
            name: name.to_string(),
            total,
            quantity: dec!(1),
            unit_price: Decimal::from(total) / Decimal::from(100),
            attributes: None,
        }
    }

    #[test]
    fn prorate_rounds_half_away_from_zero_without_float_drift() {
        assert_eq!(prorate(3100, Some(DayFraction::new(15, 31))), 1500);
        assert_eq!(prorate(3100, Some(DayFraction::new(10, 31))), 1000);
        assert_eq!(prorate(5, Some(DayFraction::new(1, 2))), 3); // 2.5 rounds away from zero
        // One day of a 28-day February on 0.70 is 2.5 cents: the f64 factor billed 2 (OQ14).
        assert_eq!(prorate(70, Some(DayFraction::new(1, 28))), 3);
        assert_eq!(prorate(2500, None), 2500);
        assert_eq!(prorate(-100, Some(DayFraction::new(1, 2))), 0); // negatives clamp to zero
    }

    #[test]
    fn usage_sublines_invariant_no_proration() {
        let sublines = vec![subline("a", 1_000), subline("b", 2_345), subline("c", 777)];
        let line = PricedLine::from_usage_sublines(sublines, None);
        let sum: i64 = line.sublines.iter().map(|s| s.total).sum();
        assert_eq!(line.total as i64, sum);
    }

    #[test]
    fn usage_sublines_invariant_with_proration() {
        // 0.37 proration introduces per-subline rounding drift
        let sublines = vec![
            subline("a", 1_000),
            subline("b", 2_345),
            subline("c", 777),
            subline("d", 133),
        ];
        let line = PricedLine::from_usage_sublines(sublines, Some(DayFraction::new(37, 100)));
        let sum: i64 = line.sublines.iter().map(|s| s.total).sum();
        assert_eq!(
            line.total as i64, sum,
            "line.total ({}) != Σ sublines.total ({})",
            line.total, sum
        );
    }

    #[test]
    fn usage_sublines_invariant_factor_one() {
        let sublines = vec![subline("a", 1_000), subline("b", 2_345)];
        let line = PricedLine::from_usage_sublines(sublines, Some(DayFraction::WHOLE));
        let sum: i64 = line.sublines.iter().map(|s| s.total).sum();
        assert_eq!(line.total as i64, sum);
    }

    #[test]
    fn a_pack_grants_its_amount_per_unit_in_minor_units() {
        assert_eq!(pack_credit_minor(dec!(50), dec!(3), 2), Ok(15_000));
        assert_eq!(pack_credit_minor(dec!(5000), dec!(1), 0), Ok(5_000));
        assert_eq!(pack_credit_minor(dec!(1.234), dec!(2), 3), Ok(2_468));
    }

    #[test]
    fn a_fractional_or_negative_grant_is_refused() {
        assert!(pack_credit_minor(dec!(0.005), dec!(1), 2).is_err());
        assert!(pack_credit_minor(dec!(-1), dec!(1), 2).is_err());
    }

    fn convert(rounding: UnitRounding, units: Decimal) -> Decimal {
        converted_units(Some(1_000_000), Some(rounding), units)
    }

    #[test]
    fn rounding_none_keeps_the_fraction() {
        assert_eq!(convert(UnitRounding::None, dec!(9_004_601)), dec!(9.004601));
        assert_eq!(
            converted_units(Some(1_000_000), None, dec!(9_004_601)),
            dec!(9.004601)
        );
    }

    #[test]
    fn rounding_up_takes_the_next_whole_unit() {
        assert_eq!(convert(UnitRounding::Up, dec!(9_004_601)), dec!(10));
        assert_eq!(convert(UnitRounding::Up, dec!(9_000_000)), dec!(9));
        assert_eq!(convert(UnitRounding::Up, dec!(1)), dec!(1));
    }

    #[test]
    fn rounding_down_drops_the_fraction() {
        assert_eq!(convert(UnitRounding::Down, dec!(9_999_999)), dec!(9));
        assert_eq!(convert(UnitRounding::Down, dec!(999_999)), dec!(0));
    }

    #[test]
    fn rounding_nearest_rounds_halves_away_from_zero() {
        assert_eq!(convert(UnitRounding::Nearest, dec!(9_499_999)), dec!(9));
        assert_eq!(convert(UnitRounding::Nearest, dec!(9_500_000)), dec!(10));
        assert_eq!(convert(UnitRounding::Nearest, dec!(8_500_000)), dec!(9));
        assert_eq!(convert(UnitRounding::Nearest, dec!(-2_500_000)), dec!(-3));
    }

    #[test]
    fn rounding_to_halves_and_tenths() {
        assert_eq!(convert(UnitRounding::NearestHalf, dec!(9_240_000)), dec!(9));
        assert_eq!(
            convert(UnitRounding::NearestHalf, dec!(9_250_000)),
            dec!(9.5)
        );
        assert_eq!(
            convert(UnitRounding::NearestDecile, dec!(9_050_000)),
            dec!(9.1)
        );
    }

    #[test]
    fn no_or_zero_factor_leaves_usage_as_is() {
        let up = Some(UnitRounding::Up);
        assert_eq!(converted_units(None, up, dec!(1.5)), dec!(1.5));
        assert_eq!(converted_units(Some(0), up, dec!(1.5)), dec!(1.5));
    }

    #[test]
    fn slots_are_billed_within_their_bounds() {
        assert_eq!(billed_slots(3, Some(5), None), 5);
        assert_eq!(billed_slots(30, None, Some(10)), 10);
        assert_eq!(billed_slots(7, Some(1), Some(10)), 7);
    }
}
