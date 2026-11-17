use std::collections::{HashMap, HashSet};

use common_utils::decimals::{round_billable_quantity, round_quantity};
use common_utils::integers::{ToNonNegativeU64, only_positive, only_positive_decimal};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::{
    GroupedUsage, PricedLine, PricingError, SubLine, SubLineAttributes, UsageModel, fees,
    simple_prorated, to_minor,
};

/// Whether usage carrying `dimensions` falls in the matrix cell of `dimension1` (and
/// `dimension2`), each a key and its value.
pub fn cell_matches(
    dimension1: (&str, &str),
    dimension2: Option<(&str, &str)>,
    dimensions: &HashMap<String, String>,
) -> bool {
    let carries =
        |(key, value): (&str, &str)| dimensions.get(key).map(String::as_str) == Some(value);
    carries(dimension1) && dimension2.is_none_or(carries)
}

/// The lines a usage fee bills for the usage of one window; shared by the invoice and the live
/// credit projection so both price identically. `unit_price_override` is the per-group unit price
/// an operator pinned on a draft line.
pub fn usage_lines(
    model: &UsageModel,
    usage: &[GroupedUsage],
    precision: u8,
    unit_price_override: impl Fn(&HashMap<String, String>) -> Option<Decimal>,
) -> Result<Vec<PricedLine>, PricingError> {
    let mut lines: Vec<PricedLine> = vec![];
    match model {
        UsageModel::Matrix { rates } => {
            // First, identify matrix dimension keys
            let mut matrix_dimension_keys = HashSet::new();
            for rate in rates {
                matrix_dimension_keys.insert(&rate.dimension1.key);
                if let Some(dimension2) = &rate.dimension2 {
                    matrix_dimension_keys.insert(&dimension2.key);
                }
            }

            // Group usage data by non-matrix dimensions (usage_group_key dimensions)
            let mut groups: HashMap<String, (HashMap<String, String>, Vec<&GroupedUsage>)> =
                HashMap::new();

            for grouped_usage in usage {
                let group_dimensions: HashMap<String, String> = grouped_usage
                    .dimensions
                    .iter()
                    .filter(|(key, _)| !matrix_dimension_keys.contains(key))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                // Create a stable string key for grouping
                let mut sorted_group_keys: Vec<_> = group_dimensions.iter().collect();
                sorted_group_keys.sort_by_key(|(k, _)| *k);
                let group_key = sorted_group_keys
                    .iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join("|");

                groups
                    .entry(group_key)
                    .or_insert_with(|| (group_dimensions.clone(), Vec::new()))
                    .1
                    .push(grouped_usage);
            }

            // Create one line item per group with matrix sublines
            for (_, (group_dimensions, group_usage_data)) in groups {
                let mut sublines = vec![];

                for rate in rates {
                    // Find usage data that matches this matrix rate within this group
                    let matching_usage = group_usage_data
                        .iter()
                        .find(|usage_data| rate.matches(&usage_data.dimensions));

                    if let Some(usage_data) = matching_usage {
                        let quantity = round_billable_quantity(
                            usage_data.value,
                            rate.per_unit_price,
                            precision,
                        );

                        if quantity > Decimal::ZERO {
                            let price_total = rate.per_unit_price * quantity;

                            let price_cents = only_positive(to_minor(price_total, precision)?);

                            if price_cents > 0 {
                                // we concat rate.dimension1.value and rate.dimension2.value (if defined), separated by a comma
                                let name = format!(
                                    "{}{}",
                                    rate.dimension1.value,
                                    rate.dimension2
                                        .as_ref()
                                        .map(|d| format!(",{}", d.value))
                                        .unwrap_or_default()
                                );
                                sublines.push(SubLine {
                                    name,
                                    total: price_cents as i64,
                                    quantity,
                                    unit_price: rate.per_unit_price,
                                    attributes: Some(SubLineAttributes::Matrix {
                                        dimension1_key: rate.dimension1.key.clone(),
                                        dimension1_value: rate.dimension1.value.clone(),
                                        dimension2_key: rate
                                            .dimension2
                                            .as_ref()
                                            .map(|d| d.key.clone()),
                                        dimension2_value: rate
                                            .dimension2
                                            .as_ref()
                                            .map(|d| d.value.clone()),
                                    }),
                                });
                            }
                        }
                    }
                }

                // Only create a line item if there are sublines for this group
                if !sublines.is_empty() {
                    let mut line = PricedLine::from_usage_sublines(sublines, None);

                    line.dimensions = if group_dimensions.is_empty() {
                        None
                    } else {
                        Some(group_dimensions)
                    };

                    lines.push(line);
                }
            }
        }
        model => {
            // Round so aggregation float noise (e.g. a summed 5524.098500000001)
            // never reaches the invoice.
            let qty_scale = model.billable_quantity_scale(precision);

            // Handle grouped usage data - create separate line items for each group
            for grouped_usage in usage {
                let usage_units = round_quantity(grouped_usage.value, qty_scale);

                // Skip zero usage
                if usage_units <= Decimal::ZERO {
                    continue;
                }

                let mut line = match model {
                    UsageModel::PerUnit { rate } => {
                        let effective_rate =
                            unit_price_override(&grouped_usage.dimensions).unwrap_or(*rate);
                        simple_prorated(&effective_rate, &usage_units, None, precision)?
                    }
                    UsageModel::Tiered { tiers, .. } => {
                        fees::tiered(usage_units, tiers, precision)?
                    }
                    UsageModel::Volume { tiers, .. } => {
                        fees::volume(usage_units, tiers, precision)?
                    }
                    UsageModel::Package { block_size, rate } => {
                        if *block_size == 0 {
                            return Err(PricingError::InvalidPrice);
                        }
                        // TODO we want some additional data in the frontend to display that "x$ per 20", total usage and block usage
                        let package_size_decimal = Decimal::from(*block_size);
                        let total_packages = (usage_units / package_size_decimal).ceil();

                        let price_total = total_packages * *rate;

                        PricedLine::from_usage_sublines(
                            vec![SubLine {
                                name: "Package".to_string(),
                                total: to_minor(price_total, precision)?,
                                quantity: total_packages,
                                unit_price: *rate,
                                attributes: Some(SubLineAttributes::Package {
                                    raw_usage: usage_units,
                                }),
                            }],
                            None,
                        )
                    }
                    UsageModel::Matrix { .. } => unreachable!(),
                };
                // Store group_by dimensions for later use in line name generation
                line.dimensions = Some(grouped_usage.dimensions.clone());
                lines.push(line);
            }
        }
    }
    Ok(lines)
}

/// The overage lines of a capacity fee for the usage of one window, past `included` units.
pub fn capacity_overage_lines(
    included: Decimal,
    overage_rate: &Decimal,
    usage: &[GroupedUsage],
    precision: u8,
) -> Result<Vec<PricedLine>, PricingError> {
    let mut lines: Vec<PricedLine> = vec![];
    if overage_rate > &Decimal::ZERO {
        for usage_data in usage {
            let overage_units = usage_data
                .value
                .checked_sub(included)
                .map(only_positive_decimal)
                .map(|units| round_billable_quantity(units, *overage_rate, precision))
                .ok_or(PricingError::OutOfRange)?;

            // Multiplied before rounding, once, so a sub-minor-unit rate still bills.
            let overage_total = overage_rate
                .checked_mul(overage_units)
                .and_then(|total| {
                    total.checked_mul(Decimal::from(10_i64.pow(u32::from(precision))))
                })
                .and_then(|minor| {
                    minor
                        .round_dp_with_strategy(
                            0,
                            rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                        )
                        .to_i64()
                })
                .ok_or(PricingError::OutOfRange)?;

            lines.push(PricedLine {
                quantity: None,
                unit_price: None,
                total: overage_total.to_non_negative_u64(),
                is_prorated: false,
                sublines: vec![SubLine {
                    name: "Overage".to_string(),
                    total: overage_total,
                    quantity: overage_units,
                    unit_price: *overage_rate,
                    attributes: None,
                }],
                dimensions: Some(usage_data.dimensions.clone()),
            });
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn overage_total(rate: Decimal, included: u64, used: Decimal, precision: u8) -> (i64, u64) {
        let usage = [GroupedUsage {
            value: used,
            dimensions: HashMap::new(),
        }];
        let lines =
            capacity_overage_lines(Decimal::from(included), &rate, &usage, precision).unwrap();
        assert_eq!(lines.len(), 1);
        (lines[0].sublines[0].total, lines[0].total)
    }

    #[test]
    fn capacity_overage_rounds_once_after_multiplying() {
        // 0.4 credits x 100 at precision 0: 40, where rounding the rate first billed 0.
        assert_eq!(overage_total(dec!(0.4), 0, dec!(100), 0), (40, 40));
        // 0.5 x 3 = 1.5 rounds half away to 2, where the rate rounded to 1 first billed 3.
        assert_eq!(overage_total(dec!(0.5), 10, dec!(13), 0), (2, 2));
        // A sub-cent money rate: $0.004 x 1000 is $4.00, not $0.
        assert_eq!(overage_total(dec!(0.004), 0, dec!(1000), 2), (400, 400));
        // Fractional overage is billed, not truncated: 10.5 x $0.02 = $0.21.
        assert_eq!(overage_total(dec!(0.02), 5, dec!(15.5), 2), (21, 21));
        assert_eq!(overage_total(dec!(0.02), 20, dec!(15.5), 2), (0, 0));
    }

    fn five_units() -> [GroupedUsage; 1] {
        [GroupedUsage {
            value: dec!(5),
            dimensions: HashMap::new(),
        }]
    }

    /// A package of no units prices nothing it could bill: an error, never a division by zero.
    /// Rules: R-107
    #[test]
    fn a_package_with_no_block_size_is_an_error() {
        let package = UsageModel::Package {
            block_size: 0,
            rate: dec!(1),
        };
        assert_eq!(
            usage_lines(&package, &five_units(), 2, |_| None),
            Err(PricingError::InvalidPrice)
        );
    }

    /// Two tiers starting on the same unit leave the price of that unit ambiguous: an error, for
    /// graduated and volume tiers alike.
    /// Rules: R-107
    #[test]
    fn tiers_starting_on_the_same_unit_are_an_error() {
        let tier = |first_unit, rate| super::super::Tier {
            first_unit,
            rate,
            flat_fee: None,
            flat_cap: None,
        };
        let tiers = vec![tier(0, dec!(1)), tier(0, dec!(0.5))];
        for model in [
            UsageModel::Tiered {
                tiers: tiers.clone(),
                block_size: None,
            },
            UsageModel::Volume {
                tiers: tiers.clone(),
                block_size: None,
            },
        ] {
            assert_eq!(
                usage_lines(&model, &five_units(), 2, |_| None),
                Err(PricingError::InvalidPrice),
                "{model:?}"
            );
        }
    }

    #[test]
    fn a_capacity_overage_beyond_the_supported_range_is_refused() {
        let usage = [GroupedUsage {
            value: Decimal::MAX,
            dimensions: HashMap::new(),
        }];
        assert!(capacity_overage_lines(Decimal::ZERO, &dec!(10), &usage, 2).is_err());
    }
}
