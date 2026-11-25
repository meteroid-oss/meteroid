use std::collections::HashMap;

use common_utils::decimals::{round_billable_quantity, round_quantity};
use common_utils::integers::only_positive_decimal;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::{
    GroupedUsage, PricedLine, PricingError, SubLine, SubLineAttributes, UsageModel, fees,
    simple_prorated, to_minor,
};

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
        UsageModel::RateCard(card) => lines.extend(rated_lines(card, usage, precision)?),
        model => {
            // Round so aggregation float noise (e.g. a summed 5524.098500000001)
            // never reaches the invoice.
            let qty_scale = model.billable_quantity_scale(precision);

            // Handle grouped usage data - create separate line items for each group
            for grouped_usage in usage {
                let usage_units = round_quantity(grouped_usage.value, qty_scale);

                // Per-unit usage nets negative (e.g. a share of revenue net of credit notes): the
                // line credits the invoice. Every other model bills only positive usage.
                let signed = matches!(model, UsageModel::PerUnit { .. });
                if usage_units.is_zero() || (usage_units.is_sign_negative() && !signed) {
                    continue;
                }

                let mut line = match model {
                    UsageModel::PerUnit { rate } => {
                        let effective_rate =
                            unit_price_override(&grouped_usage.dimensions).unwrap_or(*rate);
                        let total = to_minor(
                            effective_rate
                                .checked_mul(usage_units)
                                .ok_or(PricingError::OutOfRange)?,
                            precision,
                        )?;
                        PricedLine {
                            total,
                            ..simple_prorated(&effective_rate, &usage_units, None, precision)?
                        }
                    }
                    _ => quantity_line(model, usage_units, precision)?,
                };
                // Store group_by dimensions for later use in line name generation
                line.dimensions = Some(grouped_usage.dimensions.clone());
                lines.push(line);
            }
        }
    }
    Ok(lines)
}

/// The lines of usage rated by dimension (a card's entries on `card.on`, or a price's own
/// rates): one per group of the dimensions the rates do not key, a subline per row its entry
/// rates, each line rounded once (R-130, R-132, R-136).
fn rated_lines(
    card: &super::CardRates,
    usage: &[GroupedUsage],
    precision: u8,
) -> Result<Vec<PricedLine>, PricingError> {
    use std::collections::BTreeMap;
    if card.unit_size <= Decimal::ZERO {
        return Err(PricingError::InvalidPrice);
    }
    let on = card.on;
    let mut groups: BTreeMap<BTreeMap<String, String>, Vec<(SubLine, Decimal)>> = BTreeMap::new();
    for row in usage {
        let dimensions: BTreeMap<String, String> = row
            .dimensions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let group: BTreeMap<String, String> = dimensions
            .iter()
            .filter(|(key, _)| !card.dimensions.contains(key))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let Some(card_rate) =
            crate::rate_card::resolve(&card.dimensions, &card.entries, &dimensions, on)
        else {
            if card.ignore_unrated || row.value.is_zero() {
                continue;
            }
            return Err(PricingError::Unrated);
        };
        let scaled = card_rate
            .checked_mul(card.multiplier)
            .ok_or(PricingError::OutOfRange)?;
        let rate = if card.unit_size == Decimal::ONE {
            scaled
        } else {
            scaled
                .checked_div(card.unit_size)
                .ok_or(PricingError::OutOfRange)?
        };
        let quantity = round_billable_quantity(row.value, rate, precision);
        // Divided last: a rate per 1M units is exact on the quantity, never on a rounded unit rate.
        let exact = scaled
            .checked_mul(quantity)
            .and_then(|amount| amount.checked_div(card.unit_size))
            .ok_or(PricingError::OutOfRange)?;
        if quantity <= Decimal::ZERO || exact <= Decimal::ZERO {
            continue;
        }
        let name = card
            .dimensions
            .iter()
            .filter_map(|key| dimensions.get(key).cloned())
            .collect::<Vec<_>>()
            .join(",");
        let subline = SubLine {
            name,
            total: 0,
            quantity,
            unit_price: rate,
            attributes: keyed_values(&card.dimensions, &dimensions),
        };
        groups.entry(group).or_default().push((subline, exact));
    }
    let mut lines = Vec::new();
    for (group, sublines) in groups {
        let mut line = PricedLine::from_exact_sublines(sublines, precision)?;
        if line.total > 0 {
            line.dimensions = (!group.is_empty()).then(|| group.into_iter().collect());
            lines.push(line);
        }
    }
    Ok(lines)
}

/// The values a subline of rates keyed by one or two `keys` is the usage of; none past two.
fn keyed_values(
    keys: &[String],
    dimensions: &std::collections::BTreeMap<String, String>,
) -> Option<SubLineAttributes> {
    let (first, second) = match keys {
        [first] => (first, None),
        [first, second] => (first, Some(second)),
        _ => return None,
    };
    let second = second.and_then(|key| dimensions.get(key).map(|value| (key, value)));
    Some(SubLineAttributes::Matrix {
        dimension1_key: first.clone(),
        dimension1_value: dimensions.get(first)?.clone(),
        dimension2_key: second.map(|(key, _)| key.clone()),
        dimension2_value: second.map(|(_, value)| value.clone()),
    })
}

/// What `units` of one quantity cost under `model`: a subline per tier or package reached. Rates
/// by dimension price dimensions, never one quantity.
pub fn quantity_line(
    model: &UsageModel,
    units: Decimal,
    precision: u8,
) -> Result<PricedLine, PricingError> {
    match model {
        UsageModel::PerUnit { rate } => simple_prorated(rate, &units, None, precision),
        UsageModel::Tiered { tiers, .. } => fees::tiered(units, tiers, precision),
        UsageModel::Volume { tiers, .. } => fees::volume(units, tiers, precision),
        UsageModel::Package { block_size, rate } => {
            if *block_size == 0 {
                return Err(PricingError::InvalidPrice);
            }
            let packages = (units / Decimal::from(*block_size)).ceil();
            Ok(PricedLine::from_usage_sublines(
                vec![SubLine {
                    name: "Package".to_string(),
                    total: to_minor(packages * *rate, precision)?,
                    quantity: packages,
                    unit_price: *rate,
                    attributes: Some(SubLineAttributes::Package { raw_usage: units }),
                }],
                None,
            ))
        }
        UsageModel::RateCard(_) => Err(PricingError::InvalidPrice),
    }
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
            // The usage read is rounded, never the allowance: a prorated one is exact, and the
            // units past it are shown rounded but billed exactly (R-001).
            let past = round_billable_quantity(usage_data.value, *overage_rate, precision)
                .checked_sub(included)
                .map(only_positive_decimal)
                .ok_or(PricingError::OutOfRange)?;
            let overage_units = round_billable_quantity(past, *overage_rate, precision);

            // Multiplied before rounding, once, so a sub-minor-unit rate still bills.
            let overage_total = overage_rate
                .checked_mul(past)
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
                total: overage_total.max(0),
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

    fn overage_total(rate: Decimal, included: u64, used: Decimal, precision: u8) -> (i64, i64) {
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
    fn per_unit_usage_netting_negative_credits_the_invoice() {
        let usage = [GroupedUsage {
            value: dec!(-12_000),
            dimensions: HashMap::new(),
        }];
        let price = |model: UsageModel| -> Vec<i64> {
            usage_lines(&model, &usage, 2, |_| None)
                .unwrap()
                .iter()
                .map(|line| line.total)
                .collect()
        };
        assert_eq!(
            price(UsageModel::PerUnit { rate: dec!(0.005) }),
            vec![-6_000]
        );
        assert!(
            price(UsageModel::Package {
                block_size: 100,
                rate: dec!(1)
            })
            .is_empty()
        );
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

    /// A capacity of 100 units for a year, cancelled after 3 of its 365 days, includes 300/365
    /// units: 6 used at 0.25 owe (6 − 300/365) × 25 = 129.45, so 129, where the units past the
    /// allowance read 5.18 first and billed 129.5, so 130.
    /// Rules: R-001, R-002
    #[test]
    fn a_prorated_allowance_is_billed_past_exactly_rounded_once() {
        let usage = [GroupedUsage {
            value: dec!(6),
            dimensions: HashMap::new(),
        }];
        let included = Decimal::from(100) * Decimal::from(3) / Decimal::from(365);
        let lines = capacity_overage_lines(included, &dec!(0.25), &usage, 2).unwrap();
        assert_eq!((lines[0].total, lines[0].sublines[0].total), (129, 129));
        assert_eq!(lines[0].sublines[0].quantity, dec!(5.18));
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

    /// Rates a price holds for `cells` models, one unit of each used.
    fn own_rates_of(cells: usize, rate: Decimal) -> (UsageModel, Vec<GroupedUsage>) {
        use crate::rate_card::{Entry, Pattern};
        let model = UsageModel::RateCard(super::super::CardRates {
            dimensions: vec!["model".to_string()],
            entries: (0..cells)
                .map(|i| Entry {
                    key: vec![Pattern::Exact(format!("m{i}"))],
                    effective_from: chrono::NaiveDate::MIN,
                    rate: Some(rate),
                })
                .collect(),
            unit_size: Decimal::ONE,
            multiplier: Decimal::ONE,
            ignore_unrated: true,
            on: chrono::NaiveDate::MIN,
        });
        let usage = (0..cells)
            .map(|i| GroupedUsage {
                value: dec!(1),
                dimensions: HashMap::from([("model".to_string(), format!("m{i}"))]),
            })
            .collect();
        (model, usage)
    }

    /// 300 keys of one unit at $0.004 bill $1.20 and at $0.006 $1.80, where rounding each key
    /// billed $0.00 and $3.00. The sublines add up to the line.
    /// Rules: R-130
    #[test]
    fn rates_by_dimension_are_rounded_once_over_their_keys() {
        for (price, total) in [(dec!(0.004), 120), (dec!(0.006), 180)] {
            let (model, usage) = own_rates_of(300, price);
            let lines = usage_lines(&model, &usage, 2, |_| None).unwrap();
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].total, total);
            assert_eq!(lines[0].sublines.len(), 300);
            let sum: i64 = lines[0].sublines.iter().map(|s| s.total).sum();
            assert_eq!(sum, total);
        }
    }

    /// Rates whose keys come to less than a minor unit bill no line.
    /// Rules: R-130
    #[test]
    fn rates_below_a_minor_unit_bill_nothing() {
        let (model, usage) = own_rates_of(1, dec!(0.004));
        assert!(usage_lines(&model, &usage, 2, |_| None).unwrap().is_empty());
    }

    /// A subline of one or two keyed dimensions names their values, as a matrix cell's did.
    #[test]
    fn a_subline_names_the_values_its_rate_keys() {
        let usage = [row("a/x", "p1", dec!(1000000))];
        let lines = usage_lines(&UsageModel::RateCard(card(false)), &usage, 2, |_| None).unwrap();
        assert_eq!(
            lines[0].sublines[0].attributes,
            Some(SubLineAttributes::Matrix {
                dimension1_key: "model".to_string(),
                dimension1_value: "a/x".to_string(),
                dimension2_key: Some("token_type".to_string()),
                dimension2_value: Some("input".to_string()),
            })
        );
    }

    fn card(ignore_unrated: bool) -> super::super::CardRates {
        use crate::rate_card::{Entry, Pattern};
        let entry = |model: &str, rate| Entry {
            key: vec![Pattern::parse(model), Pattern::parse("input")],
            effective_from: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            rate: Some(rate),
        };
        super::super::CardRates {
            dimensions: vec!["model".into(), "token_type".into()],
            entries: vec![entry("a/*", dec!(3)), entry("b", dec!(10))],
            unit_size: dec!(1000000),
            multiplier: dec!(0.8),
            ignore_unrated,
            on: chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
        }
    }

    fn row(model: &str, project: &str, value: Decimal) -> GroupedUsage {
        GroupedUsage {
            value,
            dimensions: HashMap::from([
                ("model".to_string(), model.to_string()),
                ("token_type".to_string(), "input".to_string()),
                ("project".to_string(), project.to_string()),
            ]),
        }
    }

    /// 1M tokens of a/x at 3 and 1M of b at 10 per 1M, 20% off: 2.40 + 8.00 in one project's
    /// line, rounded once; another project bills its own line.
    /// Rules: R-132
    #[test]
    fn rate_card_usage_is_rated_by_entry_per_group_and_rounded_once() {
        let usage = [
            row("a/x", "p1", dec!(1000000)),
            row("b", "p1", dec!(1000000)),
            row("b", "p2", dec!(1)),
        ];
        let lines = usage_lines(&UsageModel::RateCard(card(false)), &usage, 2, |_| None).unwrap();
        let p1 = lines
            .iter()
            .find(|l| l.dimensions.as_ref().unwrap()["project"] == "p1")
            .unwrap();
        assert_eq!(p1.total, 1040);
        assert_eq!(p1.sublines.len(), 2);
        assert_eq!(lines.len(), 1, "p2's 0.000008 is below a minor unit");
    }

    /// Rules: R-132
    #[test]
    fn unrated_usage_holds_unless_the_card_ignores_it() {
        let usage = [row("c", "p1", dec!(10))];
        assert_eq!(
            usage_lines(&UsageModel::RateCard(card(false)), &usage, 2, |_| None),
            Err(PricingError::Unrated)
        );
        assert!(
            usage_lines(&UsageModel::RateCard(card(true)), &usage, 2, |_| None)
                .unwrap()
                .is_empty()
        );
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
