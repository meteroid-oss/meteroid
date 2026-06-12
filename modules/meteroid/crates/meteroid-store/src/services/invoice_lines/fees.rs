use crate::StoreResult;
use crate::domain::{Period, SubLineAttributes, SubLineItem, TierRow, UsagePricingModel};
use crate::errors::StoreError;
use crate::services::invoice_lines::component::InvoiceLineInner;
use crate::utils::local_id::LocalId;
use common_domain::ids::BillableMetricId;
use common_utils::decimals::ToSubunit;
use common_utils::integers::ToNonNegativeU64;
use error_stack::{Report, ResultExt};
use rust_decimal::Decimal;

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
fn tiered_charges(usage_units: Decimal, tiers: &[TierRow]) -> Vec<TierCharge> {
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
fn volume_charge(usage_units: Decimal, tiers: &[TierRow]) -> Option<VolumeCharge> {
    let mut sorted_rows = tiers.to_owned();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut iter = sorted_rows.iter().peekable();
    while let Some(tier) = iter.next() {
        let last_unit: Option<u64> = iter.peek().map(|row| row.first_unit - 1);

        if usage_units >= Decimal::from(tier.first_unit)
            && last_unit.is_none_or(|l| usage_units <= Decimal::from(l))
        {
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

pub fn compute_volume_price(
    usage_units: Decimal,
    tiers: &[TierRow],
    period: Period,
    precision: u8,
    metric_id: BillableMetricId,
    _block_size: &Option<u64>,
) -> StoreResult<InvoiceLineInner> {
    let charge = volume_charge(usage_units, tiers);
    let amount = charge.as_ref().map_or(Decimal::ZERO, |c| c.amount);
    let unit_price = charge.as_ref().map_or(Decimal::ZERO, |c| c.unit_price);
    let attributes = charge.as_ref().map(|c| SubLineAttributes::Volume {
        first_unit: c.first_unit,
        last_unit: c.last_unit,
        flat_cap: c.flat_cap,
        flat_fee: c.flat_fee,
    });

    let total = amount
        .to_subunit_opt(precision)
        .ok_or(Report::new(StoreError::InvalidDecimal))
        .attach("Failed to convert line total to subunit")?;

    Ok(InvoiceLineInner {
        quantity: Some(usage_units),
        unit_price: None,
        total: total.to_non_negative_u64(),
        period,
        custom_line_name: None,
        is_prorated: false,
        sublines: vec![SubLineItem {
            local_id: LocalId::no_prefix(),
            name: "Volume".to_string(),
            total,
            quantity: usage_units,
            unit_price,
            attributes,
        }],
        metric_id: Some(metric_id),
        group_by_dimensions: None,
    })
}

pub fn compute_tier_price(
    usage_units: Decimal,
    tiers: &[TierRow],
    period: Period,
    precision: u8,
    metric_id: BillableMetricId,
    _block_size: &Option<u64>,
) -> StoreResult<InvoiceLineInner> {
    let charges = tiered_charges(usage_units, tiers);

    let mut subtotal = Decimal::ZERO;
    let mut sub_lines = Vec::with_capacity(charges.len());

    for charge in &charges {
        subtotal += charge.amount;
        sub_lines.push(SubLineItem {
            local_id: LocalId::no_prefix(),
            name: format!(
                "{}-{} tier",
                charge.first_unit,
                charge.last_unit.map_or("∞".to_string(), |s| s.to_string())
            ),
            total: charge
                .amount
                .to_subunit_opt(precision)
                .ok_or(Report::new(StoreError::InvalidDecimal))
                .attach("Failed to convert subline total to subunit")?,
            quantity: charge.units,
            unit_price: charge.unit_price,
            attributes: Some(SubLineAttributes::Tiered {
                first_unit: charge.first_unit,
                last_unit: charge.last_unit,
                flat_cap: charge.flat_cap,
                flat_fee: charge.flat_fee,
            }),
        });
    }

    Ok(InvoiceLineInner {
        quantity: Some(usage_units),
        unit_price: None,
        total: subtotal
            .to_subunit_opt(precision)
            .ok_or(Report::new(StoreError::InvalidDecimal))
            .attach("Failed to convert subline total to subunit")?
            .to_non_negative_u64(),
        period,
        custom_line_name: None,
        is_prorated: false,
        sublines: sub_lines,
        metric_id: Some(metric_id),
        group_by_dimensions: None,
    })
}

/// Prices a usage quantity under a pricing model, reusing the invoicing math so
/// the result matches what would be billed. Returns `None` for Matrix, which
/// needs per-dimension quantities. Display/estimation only.
pub fn compute_usage_price(
    model: &UsagePricingModel,
    usage_units: Decimal,
    currency: &str,
) -> StoreResult<Option<Decimal>> {
    let precision = rusty_money::iso::find(currency)
        .map(|c| c.exponent as u8)
        .unwrap_or(2);

    let amount = match model {
        UsagePricingModel::PerUnit { rate } => rate * usage_units,
        UsagePricingModel::Package { block_size, rate } => {
            if *block_size == 0 {
                return Ok(Some(Decimal::ZERO));
            }
            (usage_units / Decimal::from(*block_size)).ceil() * rate
        }
        UsagePricingModel::Tiered { tiers, .. } => tiered_charges(usage_units, tiers)
            .iter()
            .map(|c| c.amount)
            .sum(),
        UsagePricingModel::Volume { tiers, .. } => {
            volume_charge(usage_units, tiers).map_or(Decimal::ZERO, |c| c.amount)
        }
        UsagePricingModel::Matrix { .. } => return Ok(None),
    };

    // Round to the currency minor unit exactly as the invoice line total does.
    let subunit = amount
        .to_subunit_opt(precision)
        .ok_or(Report::new(StoreError::InvalidDecimal))
        .attach("Failed to convert usage example amount to subunit")?
        .to_non_negative_u64();
    let divisor = Decimal::from(10u64.pow(precision as u32));
    Ok(Some(Decimal::from(subunit) / divisor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_domain::ids::BaseId;
    use rust_decimal_macros::dec;

    fn tier(
        first_unit: u64,
        rate: Decimal,
        flat_fee: Option<Decimal>,
        flat_cap: Option<Decimal>,
    ) -> TierRow {
        TierRow {
            first_unit,
            rate,
            flat_fee,
            flat_cap,
        }
    }

    fn period() -> Period {
        Period {
            start: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end: chrono::NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
        }
    }

    fn line_total(line: &InvoiceLineInner) -> Decimal {
        Decimal::from(line.total) / dec!(100)
    }

    #[test]
    fn tiered_graduated_matches_line_total() {
        let tiers = [
            tier(0, dec!(1.0), None, None),
            tier(100, dec!(0.5), None, None),
            tier(200, dec!(0.25), None, None),
        ];
        // 100*1 + 50*0.5 = 125
        let line = compute_tier_price(
            dec!(150),
            &tiers,
            period(),
            2,
            BillableMetricId::new(),
            &None,
        )
        .unwrap();
        assert_eq!(line_total(&line), dec!(125));
        assert_eq!(
            compute_usage_price(
                &UsagePricingModel::Tiered {
                    tiers: tiers.to_vec(),
                    block_size: None
                },
                dec!(150),
                "USD"
            )
            .unwrap(),
            Some(dec!(125))
        );
    }

    #[test]
    fn tiered_applies_flat_fee_and_cap() {
        // single infinite tier: 5*1 + 10 flat_fee = 15
        let with_fee = [tier(0, dec!(1.0), Some(dec!(10)), None)];
        let line = compute_tier_price(
            dec!(5),
            &with_fee,
            period(),
            2,
            BillableMetricId::new(),
            &None,
        )
        .unwrap();
        assert_eq!(line_total(&line), dec!(15));

        // 5*1 = 5, capped at 3
        let with_cap = [tier(0, dec!(1.0), None, Some(dec!(3)))];
        let line = compute_tier_price(
            dec!(5),
            &with_cap,
            period(),
            2,
            BillableMetricId::new(),
            &None,
        )
        .unwrap();
        assert_eq!(line_total(&line), dec!(3));
    }

    #[test]
    fn volume_picks_tier_matches_line_total() {
        let tiers = [
            tier(0, dec!(1.0), None, None),
            tier(100, dec!(0.5), None, None),
            tier(200, dec!(0.25), None, None),
        ];
        // 150 lands in [100,199] -> 150*0.5 = 75
        let line = compute_volume_price(
            dec!(150),
            &tiers,
            period(),
            2,
            BillableMetricId::new(),
            &None,
        )
        .unwrap();
        assert_eq!(line_total(&line), dec!(75));
        assert_eq!(
            compute_usage_price(
                &UsagePricingModel::Volume {
                    tiers: tiers.to_vec(),
                    block_size: None
                },
                dec!(150),
                "USD"
            )
            .unwrap(),
            Some(dec!(75))
        );
    }

    #[test]
    fn per_unit_and_package() {
        let per_unit = UsagePricingModel::PerUnit { rate: dec!(0.001) };
        assert_eq!(
            compute_usage_price(&per_unit, dec!(100), "USD").unwrap(),
            Some(dec!(0.10))
        );

        let package = UsagePricingModel::Package {
            block_size: 20,
            rate: dec!(5),
        };
        // ceil(45/20) = 3 -> 15
        assert_eq!(
            compute_usage_price(&package, dec!(45), "USD").unwrap(),
            Some(dec!(15))
        );
    }

    #[test]
    fn matrix_is_not_priced() {
        let matrix = UsagePricingModel::Matrix { rates: vec![] };
        assert_eq!(
            compute_usage_price(&matrix, dec!(100), "USD").unwrap(),
            None
        );
    }
}
