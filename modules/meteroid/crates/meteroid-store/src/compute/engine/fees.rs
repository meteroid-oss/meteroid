use crate::compute::engine::component::InvoiceLineInner;
use crate::compute::ComputeError;
use crate::domain::{Period, SubLineAttributes, SubLineItem, TierRow};
use crate::utils::decimals::ToSubunit;
use crate::utils::local_id::LocalId;
use rust_decimal::Decimal;

pub fn compute_volume_price(
    usage_units: Decimal,
    tiers: &Vec<TierRow>,
    period: Period,
    precision: u8,
    _block_size: &Option<u64>,
) -> Result<InvoiceLineInner, ComputeError> {
    let mut applicable_price_per_unit = Decimal::new(0, 0);
    let mut applicable_flat_fee = Decimal::new(0, 0);
    let mut applicable_flat_cap = Decimal::MAX;
    let mut sorted_rows = tiers.clone();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut iter = sorted_rows.iter().peekable();

    let mut subline_attr = None;

    // TODO block_size
    while let Some(tier) = iter.next() {
        let next_tier = iter.peek();
        let last_unit: Option<u64> = next_tier.map(|row| row.first_unit - 1);

        if usage_units >= Decimal::from(tier.first_unit)
            && last_unit
                .map(|l| usage_units <= Decimal::from(l))
                .unwrap_or(true)
        {
            applicable_price_per_unit = tier.rate;
            if let Some(flat_fee) = tier.flat_fee {
                applicable_flat_fee = flat_fee;
            }
            if let Some(flat_cap) = tier.flat_cap {
                applicable_flat_cap = flat_cap;
            }

            subline_attr = Some(SubLineAttributes::Volume {
                first_unit: tier.first_unit,
                last_unit: last_unit,
                flat_cap: tier.flat_cap,
                flat_fee: tier.flat_fee,
            });

            break;
        }
    }
    let price = usage_units * applicable_price_per_unit + applicable_flat_fee;

    let price = if price > applicable_flat_cap {
        applicable_flat_cap
    } else {
        price
    };

    Ok(InvoiceLineInner {
        quantity: Some(usage_units),
        unit_price: None,
        total: price
            .to_subunit_opt(precision)
            .ok_or_else(|| ComputeError::ConversionError)? as u64,
        period: period,
        custom_line_name: None,
        is_prorated: false,
        sublines: vec![SubLineItem {
            local_id: LocalId::no_prefix(),
            name: "Volume".to_string(),
            total: price
                .to_subunit_opt(precision)
                .ok_or_else(|| ComputeError::ConversionError)?,
            quantity: usage_units,
            unit_price: applicable_price_per_unit,
            attributes: subline_attr,
        }],
    })
}

pub fn compute_tier_price(
    usage_units: Decimal,
    tiers: &Vec<TierRow>,
    period: Period,
    precision: u8,
    _block_size: &Option<u64>,
) -> Result<InvoiceLineInner, ComputeError> {
    let mut subtotal = Decimal::new(0, 0);
    let mut remaining_usage = usage_units;

    let mut sorted_rows = tiers.clone();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut iter = sorted_rows.iter().peekable();

    let mut sub_lines = Vec::new();

    while let Some(tier) = iter.next() {
        if remaining_usage.is_zero() {
            break;
        }

        let next_tier = iter.peek();
        let last_unit: Option<u64> = next_tier.map(|row| row.first_unit);

        let tier_units = match last_unit {
            Some(last_unit) => Decimal::from(last_unit.saturating_sub(tier.first_unit)),
            None => Decimal::MAX, // Handle infinite tier
        };

        let units_in_this_tier = if remaining_usage > tier_units {
            tier_units
        } else {
            remaining_usage
        };

        let tier_price = tier.rate;

        if units_in_this_tier > Decimal::ZERO {
            let mut fee = units_in_this_tier * tier_price;
            if let Some(flat_fee) = tier.flat_fee {
                fee += flat_fee;
            }
            if let Some(cap) = tier.flat_cap {
                if fee > cap {
                    fee = cap;
                }
            }
            subtotal += fee;

            sub_lines.push(SubLineItem {
                local_id: LocalId::no_prefix(),
                name: format!(
                    "{}-{} tier",
                    tier.first_unit,
                    last_unit.map(|s| s.to_string()).unwrap_or("∞".to_string())
                ),
                total: fee
                    .to_subunit_opt(precision)
                    .ok_or_else(|| ComputeError::ConversionError)?,
                quantity: units_in_this_tier,
                unit_price: tier_price,
                attributes: Some(SubLineAttributes::Tiered {
                    first_unit: tier.first_unit,
                    last_unit: last_unit,
                    flat_cap: tier.flat_cap,
                    flat_fee: tier.flat_fee,
                }),
            });
        }
        remaining_usage -= units_in_this_tier;
    }

    Ok(InvoiceLineInner {
        quantity: Some(usage_units),
        unit_price: None,
        total: subtotal
            .to_subunit_opt(precision)
            .ok_or_else(|| ComputeError::ConversionError)? as u64,
        period: period,
        custom_line_name: None,
        is_prorated: false,
        sublines: sub_lines,
    })
}
