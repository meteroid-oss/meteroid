//! The store's price models as `billing_core::pricing` reads them, and its lines back.

use crate::StoreResult;
use crate::domain::enums::UnitConversionRoundingEnum;
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
        pricing::PricingError::Unrated => Report::new(StoreError::FailedPrecondition(
            "usage no rate of its price or rate card rates: add a rate for it, then billing resumes"
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
            // The price's own rates, rated as a card's entries are (R-136).
            UsagePricingModel::DimensionRates(rates) => {
                pricing::UsageModel::RateCard(rates.card_rates())
            }
            // The loader resolves the card's entries; unresolved, every row is unrated.
            UsagePricingModel::RateCard { multiplier, .. } => {
                pricing::UsageModel::RateCard(pricing::CardRates {
                    dimensions: Vec::new(),
                    entries: Vec::new(),
                    unit_size: Decimal::ONE,
                    multiplier: multiplier.unwrap_or(Decimal::ONE),
                    ignore_unrated: false,
                    on: chrono::NaiveDate::MIN,
                })
            }
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
/// the result matches what would be billed. Returns `None` for rates by dimension,
/// which need per-dimension quantities. Display/estimation only.
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
        let rates = UsagePricingModel::DimensionRates(rates(&[(&["*", "*"], dec!(1))]));
        assert_eq!(compute_usage_price(&rates, dec!(100), "USD").unwrap(), None);
    }

    fn rates(
        keys: &[(&[&str], rust_decimal::Decimal)],
    ) -> crate::domain::price_components::DimensionRates {
        crate::domain::price_components::DimensionRates {
            dimensions: vec!["model".to_owned(), "token_type".to_owned()],
            rates: keys
                .iter()
                .map(
                    |(key, rate)| crate::domain::price_components::DimensionRate {
                        key: key.iter().map(|p| (*p).to_owned()).collect(),
                        rate: *rate,
                        label: None,
                    },
                )
                .collect(),
            unit_size: dec!(1000000),
            on_unmatched: crate::domain::rate_cards::RateCardUnmatched::Hold,
        }
    }

    fn usage(model: &str, kind: &str, project: &str, tokens: i64) -> pricing::GroupedUsage {
        pricing::GroupedUsage {
            value: Decimal::from(tokens),
            dimensions: std::collections::HashMap::from([
                ("model".to_owned(), model.to_owned()),
                ("token_type".to_owned(), kind.to_owned()),
                ("project".to_owned(), project.to_owned()),
            ]),
        }
    }

    /// A price's own rates resolve as a card's: the most specific key wins, a wildcard rates
    /// what no other key does, rates are per `unit_size` units; a subline per usage row, a line
    /// per group of the dimensions not keyed, rounded once.
    /// Rules: R-136, R-131, R-130
    #[test]
    fn rates_in_a_price_resolve_by_specificity_per_unit_size() {
        let model = UsagePricingModel::DimensionRates(rates(&[
            (&["*", "*"], dec!(1)),
            (&["gpt-*", "*"], dec!(2)),
            (&["gpt-5", "*"], dec!(5)),
            (&["gpt-5", "output"], dec!(10)),
        ]));
        let lines = pricing::usage_lines(
            &(&model).into(),
            &[
                // 1M at 10 (exact on both), 1M at 5 (exact model), 2M at 2 (prefix), 0.5M at 1.
                usage("gpt-5", "output", "p1", 1_000_000),
                usage("gpt-5", "input", "p1", 1_000_000),
                usage("gpt-4o", "input", "p1", 2_000_000),
                usage("claude", "input", "p1", 500_000),
                usage("gpt-5", "output", "p2", 100_000),
            ],
            2,
            |_| None,
        )
        .unwrap();
        let p1 = lines
            .iter()
            .find(|line| line.dimensions.as_ref().unwrap()["project"] == "p1")
            .unwrap();
        assert_eq!(p1.total, 10_00 + 5_00 + 4_00 + 50);
        let mut rates: Vec<(String, Decimal)> = p1
            .sublines
            .iter()
            .map(|subline| (subline.name.clone(), subline.unit_price))
            .collect();
        rates.sort();
        assert_eq!(
            rates,
            vec![
                ("claude,input".to_owned(), dec!(0.000001)),
                ("gpt-4o,input".to_owned(), dec!(0.000002)),
                ("gpt-5,input".to_owned(), dec!(0.000005)),
                ("gpt-5,output".to_owned(), dec!(0.00001)),
            ]
        );
        let p2 = lines
            .iter()
            .find(|line| line.dimensions.as_ref().unwrap()["project"] == "p2")
            .unwrap();
        assert_eq!(p2.total, 1_00);
    }

    /// Usage no rate keys holds the invoice, unless the price leaves it unbilled.
    /// Rules: R-136, R-132
    #[test]
    fn usage_no_rate_of_a_price_keys_holds_unless_ignored() {
        let held = rates(&[(&["gpt-*", "*"], dec!(1))]);
        let unmatched = [usage("claude", "input", "p1", 1_000_000)];
        assert_eq!(
            pricing::usage_lines(
                &(&UsagePricingModel::DimensionRates(held.clone())).into(),
                &unmatched,
                2,
                |_| None
            ),
            Err(pricing::PricingError::Unrated)
        );
        let ignored = crate::domain::price_components::DimensionRates {
            on_unmatched: crate::domain::rate_cards::RateCardUnmatched::Ignore,
            ..held
        };
        assert!(
            pricing::usage_lines(
                &(&UsagePricingModel::DimensionRates(ignored)).into(),
                &unmatched,
                2,
                |_| None
            )
            .unwrap()
            .is_empty()
        );
    }
}
