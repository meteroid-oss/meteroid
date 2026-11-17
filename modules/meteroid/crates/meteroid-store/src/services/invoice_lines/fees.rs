//! The store's price models as `billing_core::pricing` reads them, and its lines back.

use crate::StoreResult;
use crate::domain::enums::UnitConversionRoundingEnum;
use crate::domain::price_components::{MatrixDimension, MatrixRow};
use crate::domain::{SubLineAttributes, SubLineItem, TierRow, UsagePricingModel};
use crate::errors::StoreError;
use billing_core::pricing;
use error_stack::{Report, ResultExt};
use rust_decimal::Decimal;

pub(crate) fn pricing_error(error: pricing::PricingError) -> Report<StoreError> {
    match error {
        pricing::PricingError::OutOfRange => Report::new(StoreError::InvalidDecimal),
        pricing::PricingError::InvalidPackCredit => Report::new(StoreError::InvalidArgument(
            "a credit pack must grant a non-negative amount of credit, with no more decimal places than its currency or credit unit allows".to_owned(),
        )),
        pricing::PricingError::InvalidPrice => Report::new(StoreError::InvalidArgument(
            "a package must hold at least one unit, and no two tiers may start on the same unit"
                .to_owned(),
        )),
    }
}

impl From<&TierRow> for pricing::Tier {
    fn from(row: &TierRow) -> Self {
        pricing::Tier {
            first_unit: row.first_unit,
            rate: row.rate,
            flat_fee: row.flat_fee,
            flat_cap: row.flat_cap,
        }
    }
}

impl From<&MatrixDimension> for pricing::MatrixDimension {
    fn from(dimension: &MatrixDimension) -> Self {
        pricing::MatrixDimension {
            key: dimension.key.clone(),
            value: dimension.value.clone(),
        }
    }
}

impl From<&MatrixRow> for pricing::MatrixCell {
    fn from(row: &MatrixRow) -> Self {
        pricing::MatrixCell {
            dimension1: (&row.dimension1).into(),
            dimension2: row.dimension2.as_ref().map(Into::into),
            per_unit_price: row.per_unit_price,
        }
    }
}

impl From<&UsagePricingModel> for pricing::UsageModel {
    fn from(model: &UsagePricingModel) -> Self {
        let tiers = |tiers: &[TierRow]| tiers.iter().map(Into::into).collect();
        match model {
            UsagePricingModel::PerUnit { rate } => pricing::UsageModel::PerUnit { rate: *rate },
            UsagePricingModel::Tiered {
                tiers: t,
                block_size,
            } => pricing::UsageModel::Tiered {
                tiers: tiers(t),
                block_size: *block_size,
            },
            UsagePricingModel::Volume {
                tiers: t,
                block_size,
            } => pricing::UsageModel::Volume {
                tiers: tiers(t),
                block_size: *block_size,
            },
            UsagePricingModel::Package { block_size, rate } => pricing::UsageModel::Package {
                block_size: *block_size,
                rate: *rate,
            },
            UsagePricingModel::Matrix { rates } => pricing::UsageModel::Matrix {
                rates: rates.iter().map(Into::into).collect(),
            },
        }
    }
}

impl From<&UnitConversionRoundingEnum> for pricing::UnitRounding {
    fn from(rounding: &UnitConversionRoundingEnum) -> Self {
        match rounding {
            UnitConversionRoundingEnum::Up => pricing::UnitRounding::Up,
            UnitConversionRoundingEnum::Down => pricing::UnitRounding::Down,
            UnitConversionRoundingEnum::Nearest => pricing::UnitRounding::Nearest,
            UnitConversionRoundingEnum::NearestHalf => pricing::UnitRounding::NearestHalf,
            UnitConversionRoundingEnum::NearestDecile => pricing::UnitRounding::NearestDecile,
            UnitConversionRoundingEnum::None => pricing::UnitRounding::None,
        }
    }
}

impl From<pricing::SubLineAttributes> for SubLineAttributes {
    fn from(attributes: pricing::SubLineAttributes) -> Self {
        match attributes {
            pricing::SubLineAttributes::Package { raw_usage } => {
                SubLineAttributes::Package { raw_usage }
            }
            pricing::SubLineAttributes::Tiered {
                first_unit,
                last_unit,
                flat_cap,
                flat_fee,
            } => SubLineAttributes::Tiered {
                first_unit,
                last_unit,
                flat_cap,
                flat_fee,
            },
            pricing::SubLineAttributes::Volume {
                first_unit,
                last_unit,
                flat_cap,
                flat_fee,
            } => SubLineAttributes::Volume {
                first_unit,
                last_unit,
                flat_cap,
                flat_fee,
            },
            pricing::SubLineAttributes::Matrix {
                dimension1_key,
                dimension1_value,
                dimension2_key,
                dimension2_value,
            } => SubLineAttributes::Matrix {
                dimension1_key,
                dimension1_value,
                dimension2_key,
                dimension2_value,
            },
        }
    }
}

impl From<pricing::SubLine> for SubLineItem {
    fn from(subline: pricing::SubLine) -> Self {
        SubLineItem {
            local_id: String::new(),
            name: subline.name,
            total: subline.total,
            quantity: subline.quantity,
            unit_price: subline.unit_price,
            attributes: subline.attributes.map(Into::into),
        }
    }
}

/// Prices a usage quantity under a pricing model, reusing the invoicing math so
/// the result matches what would be billed. Returns `None` for Matrix, which
/// needs per-dimension quantities. Display/estimation only.
pub fn compute_usage_price(
    model: &UsagePricingModel,
    usage_units: Decimal,
    currency: &str,
) -> StoreResult<Option<Decimal>> {
    let precision = meteroid_money::iso::find(currency)
        .map(|c| c.exponent as u8)
        .unwrap_or(2);

    pricing::usage_price(&model.into(), usage_units, precision)
        .map_err(pricing_error)
        .attach("Failed to convert usage example amount to subunit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn prices_in_the_currency_minor_unit() {
        let per_unit = UsagePricingModel::PerUnit { rate: dec!(0.001) };
        assert_eq!(
            compute_usage_price(&per_unit, dec!(100), "USD").unwrap(),
            Some(dec!(0.10))
        );
        assert_eq!(
            compute_usage_price(&per_unit, dec!(400), "JPY").unwrap(),
            Some(dec!(0))
        );
        let matrix = UsagePricingModel::Matrix { rates: vec![] };
        assert_eq!(
            compute_usage_price(&matrix, dec!(100), "USD").unwrap(),
            None
        );
    }
}
