use meteroid_store::domain::TierRow;
use rust_decimal::Decimal;

pub fn compute_volume_price(
    usage_units: Decimal,
    tiers: &Vec<TierRow>,
    _block_size: &Option<u64>,
) -> Decimal {
    let mut applicable_price_per_unit = Decimal::new(0, 0);
    let mut applicable_flat_fee = Decimal::new(0, 0);
    let mut applicable_flat_cap = Decimal::MAX;
    let mut sorted_rows = tiers.clone();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut iter = sorted_rows.iter().peekable();

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
            break;
        }
    }
    let price = usage_units * applicable_price_per_unit + applicable_flat_fee;
    if price > applicable_flat_cap {
        applicable_flat_cap
    } else {
        price
    }
}

pub fn compute_tier_price(
    usage_units: Decimal,
    tiers: &Vec<TierRow>,
    _block_size: &Option<u64>,
) -> Decimal {
    let mut subtotal = Decimal::new(0, 0);
    let mut remaining_usage = usage_units;

    let mut sorted_rows = tiers.clone();
    sorted_rows.sort_by_key(|r| r.first_unit);

    let mut iter = sorted_rows.iter().peekable();
    while let Some(tier) = iter.next() {
        if remaining_usage.is_zero() {
            break;
        }

        let next_tier = iter.peek();
        let last_unit: Option<u64> = next_tier.map(|row| row.first_unit - 1);

        let tier_units = match last_unit {
            Some(last_unit) if tier.first_unit == 0 => Decimal::from(last_unit),
            Some(last_unit) => {
                Decimal::from(last_unit.saturating_sub(tier.first_unit).saturating_add(1))
            }
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
        }
        remaining_usage -= units_in_this_tier;
    }
    subtotal
}
