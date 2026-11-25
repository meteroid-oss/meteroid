use common_utils::integers::ToNonNegativeU64;
use rust_decimal::Decimal;

use super::{PricedLine, PricingError, SubLine, SubLineAttributes, Tier, UsageModel, to_minor};

struct TierCharge {
    first_unit: u64,
    last_unit: Option<u64>,
    units: Decimal,
    unit_price: Decimal,
    flat_fee: Option<Decimal>,
    flat_cap: Option<Decimal>,
    amount: Decimal,
}

struct VolumeCharge {
    first_unit: u64,
    last_unit: Option<u64>,
    unit_price: Decimal,
    flat_fee: Option<Decimal>,
    flat_cap: Option<Decimal>,
    amount: Decimal,
}

// Graduated tiers: each tier is charged for the units that fall within it.
// TODO block_size
fn tiered_charges(usage_units: Decimal, tiers: &[Tier]) -> Vec<TierCharge> {
    let mut sorted_rows = tiers.to_owned();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut remaining_usage = usage_units;
    let mut iter = sorted_rows.iter().peekable();
    let mut charges = Vec::new();

    while let Some(tier) = iter.next() {
        if remaining_usage.is_zero() {
            break;
        }

        let last_unit: Option<u64> = iter.peek().map(|row| row.first_unit);
        let tier_units = match last_unit {
            Some(last) => Decimal::from(last.saturating_sub(tier.first_unit)),
            None => Decimal::MAX,
        };

        let units = if remaining_usage > tier_units {
            tier_units
        } else {
            remaining_usage
        };

        if units > Decimal::ZERO {
            let mut amount = units * tier.rate;
            if let Some(flat_fee) = tier.flat_fee {
                amount += flat_fee;
            }
            if let Some(cap) = tier.flat_cap
                && amount > cap
            {
                amount = cap;
            }
            charges.push(TierCharge {
                first_unit: tier.first_unit,
                last_unit,
                units,
                unit_price: tier.rate,
                flat_fee: tier.flat_fee,
                flat_cap: tier.flat_cap,
                amount,
            });
        }
        remaining_usage -= units;
    }
    charges
}

// Volume: the whole usage is charged at the rate of the single tier it lands in.
fn volume_charge(usage_units: Decimal, tiers: &[Tier]) -> Option<VolumeCharge> {
    let mut sorted_rows = tiers.to_owned();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut iter = sorted_rows.iter().peekable();
    while let Some(tier) = iter.next() {
        let next = iter.peek().map(|row| row.first_unit);
        // A bracket ends where the next one starts, so a fraction below it still lands in it.
        if usage_units >= Decimal::from(tier.first_unit)
            && next.is_none_or(|next| usage_units < Decimal::from(next))
        {
            let last_unit = next.map(|next| next.saturating_sub(1));
            let mut amount = usage_units * tier.rate;
            if let Some(flat_fee) = tier.flat_fee {
                amount += flat_fee;
            }
            if let Some(cap) = tier.flat_cap
                && amount > cap
            {
                amount = cap;
            }
            return Some(VolumeCharge {
                first_unit: tier.first_unit,
                last_unit,
                unit_price: tier.rate,
                flat_fee: tier.flat_fee,
                flat_cap: tier.flat_cap,
                amount,
            });
        }
    }
    None
}

/// Refuses tiers two of which start on the same unit: which one prices it is undefined.
fn distinct_starts(tiers: &[Tier]) -> Result<(), PricingError> {
    let mut starts: Vec<u64> = tiers.iter().map(|tier| tier.first_unit).collect();
    starts.sort_unstable();
    if starts.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(PricingError::InvalidPrice);
    }
    Ok(())
}

/// `usage_units` priced at the rate of the one tier they land in.
pub fn volume(
    usage_units: Decimal,
    tiers: &[Tier],
    precision: u8,
) -> Result<PricedLine, PricingError> {
    distinct_starts(tiers)?;
    let charge = volume_charge(usage_units, tiers);
    let amount = charge.as_ref().map_or(Decimal::ZERO, |c| c.amount);
    let unit_price = charge.as_ref().map_or(Decimal::ZERO, |c| c.unit_price);
    let attributes = charge.as_ref().map(|c| SubLineAttributes::Volume {
        first_unit: c.first_unit,
        last_unit: c.last_unit,
        flat_cap: c.flat_cap,
        flat_fee: c.flat_fee,
    });

    let total = to_minor(amount, precision)?;

    Ok(PricedLine {
        quantity: Some(usage_units),
        unit_price: None,
        total: total.max(0),
        is_prorated: false,
        sublines: vec![SubLine {
            name: "Volume".to_string(),
            total,
            quantity: usage_units,
            unit_price,
            attributes,
        }],
        dimensions: None,
    })
}

/// `usage_units` priced through graduated tiers, a subline per tier reached.
pub fn tiered(
    usage_units: Decimal,
    tiers: &[Tier],
    precision: u8,
) -> Result<PricedLine, PricingError> {
    distinct_starts(tiers)?;
    let sublines = tiered_charges(usage_units, tiers)
        .into_iter()
        .map(|charge| {
            let subline = SubLine {
                name: format!(
                    "{}-{} tier",
                    charge.first_unit,
                    charge.last_unit.map_or("∞".to_string(), |s| s.to_string())
                ),
                total: 0,
                quantity: charge.units,
                unit_price: charge.unit_price,
                attributes: Some(SubLineAttributes::Tiered {
                    first_unit: charge.first_unit,
                    last_unit: charge.last_unit,
                    flat_cap: charge.flat_cap,
                    flat_fee: charge.flat_fee,
                }),
            };
            (subline, charge.amount)
        })
        .collect();
    Ok(PricedLine {
        quantity: Some(usage_units),
        ..PricedLine::from_exact_sublines(sublines, precision)?
    })
}

/// The unrounded amount `usage_units` of usage costs under a pricing model. `None` for rates
/// by dimension, which need per-dimension quantities.
pub fn usage_amount(model: &UsageModel, usage_units: Decimal) -> Option<Decimal> {
    Some(match model {
        UsageModel::PerUnit { rate } => rate * usage_units,
        UsageModel::Package { block_size, rate } => {
            if *block_size == 0 {
                return Some(Decimal::ZERO);
            }
            (usage_units / Decimal::from(*block_size)).ceil() * rate
        }
        UsageModel::Tiered { tiers, .. } => tiered_charges(usage_units, tiers)
            .iter()
            .map(|c| c.amount)
            .sum(),
        UsageModel::Volume { tiers, .. } => {
            volume_charge(usage_units, tiers).map_or(Decimal::ZERO, |c| c.amount)
        }
        UsageModel::RateCard(_) => return None,
    })
}

/// Prices a usage quantity under a pricing model, rounded to the minor unit exactly as the
/// invoice line total is. `None` for rates by dimension. Display/estimation only.
pub fn usage_price(
    model: &UsageModel,
    usage_units: Decimal,
    precision: u8,
) -> Result<Option<Decimal>, PricingError> {
    let Some(amount) = usage_amount(model, usage_units) else {
        return Ok(None);
    };
    let subunit = to_minor(amount, precision)?.to_non_negative_u64();
    let divisor = Decimal::from(10u64.pow(precision as u32));
    Ok(Some(Decimal::from(subunit) / divisor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn tier(
        first_unit: u64,
        rate: Decimal,
        flat_fee: Option<Decimal>,
        flat_cap: Option<Decimal>,
    ) -> Tier {
        Tier {
            first_unit,
            rate,
            flat_fee,
            flat_cap,
        }
    }

    fn line_total(line: &PricedLine) -> Decimal {
        Decimal::from(line.total) / dec!(100)
    }

    /// Three tiers of $0.333 for 1, 2 and 3 units: 0.333 + 0.666 + 0.999 = 1.998, $2.00, its
    /// sublines 33 + 67 + 100, where each rounded alone added up to 2.00 only by luck.
    /// Rules: R-130
    #[test]
    fn tiered_sublines_add_up_to_the_line() {
        let tiers = [
            tier(0, dec!(0.333), None, None),
            tier(1, dec!(0.333), None, None),
            tier(3, dec!(0.333), None, None),
        ];
        let line = tiered(dec!(6), &tiers, 2).unwrap();
        assert_eq!(line.total, 200);
        let totals: Vec<i64> = line.sublines.iter().map(|s| s.total).collect();
        assert_eq!(totals.iter().sum::<i64>(), 200);
        assert_eq!(line.quantity, Some(dec!(6)));
    }

    #[test]
    fn tiered_graduated_matches_line_total() {
        let tiers = [
            tier(0, dec!(1.0), None, None),
            tier(100, dec!(0.5), None, None),
            tier(200, dec!(0.25), None, None),
        ];
        // 100*1 + 50*0.5 = 125
        let line = tiered(dec!(150), &tiers, 2).unwrap();
        assert_eq!(line_total(&line), dec!(125));
        assert_eq!(
            usage_price(
                &UsageModel::Tiered {
                    tiers: tiers.to_vec(),
                    block_size: None
                },
                dec!(150),
                2
            ),
            Ok(Some(dec!(125)))
        );
    }

    #[test]
    fn tiered_applies_flat_fee_and_cap() {
        // single infinite tier: 5*1 + 10 flat_fee = 15
        let with_fee = [tier(0, dec!(1.0), Some(dec!(10)), None)];
        assert_eq!(
            line_total(&tiered(dec!(5), &with_fee, 2).unwrap()),
            dec!(15)
        );

        // 5*1 = 5, capped at 3
        let with_cap = [tier(0, dec!(1.0), None, Some(dec!(3)))];
        assert_eq!(line_total(&tiered(dec!(5), &with_cap, 2).unwrap()), dec!(3));
    }

    #[test]
    fn volume_picks_tier_matches_line_total() {
        let tiers = [
            tier(0, dec!(1.0), None, None),
            tier(100, dec!(0.5), None, None),
            tier(200, dec!(0.25), None, None),
        ];
        // 150 lands in [100,199] -> 150*0.5 = 75
        let line = volume(dec!(150), &tiers, 2).unwrap();
        assert_eq!(line_total(&line), dec!(75));
        assert_eq!(
            usage_price(
                &UsageModel::Volume {
                    tiers: tiers.to_vec(),
                    block_size: None
                },
                dec!(150),
                2
            ),
            Ok(Some(dec!(75)))
        );
    }

    /// A volume bracket runs up to the next one's first unit, exclusive: 99.5 units under tiers
    /// from 0 at 1.00 and from 100 at 0.50 are all billed at 1.00, 9950, never 0.
    /// Rules: R-107
    #[test]
    fn volume_prices_a_fraction_between_brackets_at_the_lower_tier() {
        let tiers = [
            tier(0, dec!(1), None, None),
            tier(100, dec!(0.5), None, None),
        ];
        assert_eq!(volume(dec!(99.5), &tiers, 2).unwrap().total, 9_950);
        assert_eq!(volume(dec!(100), &tiers, 2).unwrap().total, 5_000);
        let gigabytes = [
            tier(0, dec!(0.10), None, None),
            tier(100, dec!(0.08), None, None),
        ];
        assert_eq!(volume(dec!(99.6), &gigabytes, 2).unwrap().total, 996);
        let from_one = [
            tier(1, dec!(1), None, None),
            tier(10, dec!(0.5), None, None),
        ];
        assert_eq!(volume(dec!(9.5), &from_one, 2).unwrap().total, 950);
    }

    #[test]
    fn per_unit_and_package() {
        let per_unit = UsageModel::PerUnit { rate: dec!(0.001) };
        assert_eq!(usage_price(&per_unit, dec!(100), 2), Ok(Some(dec!(0.10))));

        let package = UsageModel::Package {
            block_size: 20,
            rate: dec!(5),
        };
        // ceil(45/20) = 3 -> 15
        assert_eq!(usage_price(&package, dec!(45), 2), Ok(Some(dec!(15))));
    }

    #[test]
    fn rates_by_dimension_are_not_priced() {
        let rates = UsageModel::RateCard(super::super::CardRates {
            dimensions: vec!["model".to_owned()],
            entries: vec![],
            unit_size: dec!(1),
            multiplier: dec!(1),
            ignore_unrated: true,
            on: chrono::NaiveDate::MIN,
        });
        assert_eq!(usage_price(&rates, dec!(100), 2), Ok(None));
    }
}
