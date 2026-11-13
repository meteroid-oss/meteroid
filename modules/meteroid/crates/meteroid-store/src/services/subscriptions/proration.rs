use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::subscription_changes::{
    AddedComponent, ChangeDirection, MatchedComponent, PackGrant, ProrationLineItem,
    ProrationResult, RemovedComponent,
};
use crate::domain::subscription_components::SubscriptionFee;
use chrono::NaiveDate;
use common_utils::decimals::ToSubunit;
use rust_decimal::Decimal;

/// Per-period advance-billed amount (cents). Returns 0 for arrears/usage/onetime.
/// Mirrors the pattern from calculate_mrr.
pub fn component_advance_amount_cents(
    fee: &SubscriptionFee,
    period: &SubscriptionFeeBillingPeriod,
    precision: u8,
) -> i64 {
    if matches!(period, SubscriptionFeeBillingPeriod::OneTime) {
        return 0;
    }

    match fee {
        SubscriptionFee::Rate { rate } => rate.to_subunit_opt(precision).unwrap_or(0),
        SubscriptionFee::Recurring {
            rate,
            quantity,
            billing_type,
        } => {
            use crate::domain::enums::BillingType;
            match billing_type {
                BillingType::Advance => {
                    let total = *rate * rust_decimal::Decimal::from(*quantity);
                    total.to_subunit_opt(precision).unwrap_or(0)
                }
                BillingType::Arrears => 0,
            }
        }
        SubscriptionFee::Capacity { rate, .. } => rate.to_subunit_opt(precision).unwrap_or(0),
        SubscriptionFee::Slot {
            initial_slots,
            unit_rate,
            ..
        } => i64::from(*initial_slots) * unit_rate.to_subunit_opt(precision).unwrap_or(0),
        // A pack is never prorated on a plan change: an adjustment line funds no credit, so
        // prorating would charge without a grant or refund a charge whose credit was granted.
        SubscriptionFee::Credit { .. }
        | SubscriptionFee::OneTime { .. }
        | SubscriptionFee::Usage { .. } => 0,
    }
}

/// Total (non-prorated) amount for a one-time fee. One-time charges are billed
/// in full when added mid-period — they are never prorated.
pub fn component_onetime_amount_cents(fee: &SubscriptionFee, precision: u8) -> i64 {
    match fee {
        SubscriptionFee::OneTime { rate, quantity } => (*rate
            * rust_decimal::Decimal::from(*quantity))
        .to_subunit_opt(precision)
        .unwrap_or(0),
        _ => 0,
    }
}

/// Display instance count encoded in a fee: slot count, or recurring/one-time
/// quantity. Flat fees (Rate, Capacity) and usage are a single unit. Used so a
/// prorated adjustment-invoice line shows "N × effective_rate" instead of
/// "1 × total" for a multi-instance component. Only valid for an unscaled fee
/// (scale_fee folds the multiplier into Rate/Capacity rates, losing the count).
pub(crate) fn fee_instance_count(fee: &SubscriptionFee) -> rust_decimal::Decimal {
    use rust_decimal::Decimal;
    match fee {
        SubscriptionFee::Slot { initial_slots, .. } => Decimal::from(*initial_slots),
        SubscriptionFee::Recurring { quantity, .. } => Decimal::from(*quantity),
        SubscriptionFee::OneTime { quantity, .. } => Decimal::from(*quantity),
        SubscriptionFee::Credit { quantity, .. } => Decimal::from(*quantity),
        SubscriptionFee::Rate { .. }
        | SubscriptionFee::Capacity { .. }
        | SubscriptionFee::Usage { .. } => Decimal::ONE,
    }
}

pub use billing_core::DayFraction;

/// Nominal length, in days, of a billing period. Used to prorate components
/// whose billing cadence differs from the subscription's current period.
fn nominal_period_days(period: &SubscriptionFeeBillingPeriod) -> i64 {
    match period {
        SubscriptionFeeBillingPeriod::Monthly => 30,
        SubscriptionFeeBillingPeriod::Quarterly => 91,
        SubscriptionFeeBillingPeriod::Semiannual => 182,
        SubscriptionFeeBillingPeriod::Annual => 365,
        SubscriptionFeeBillingPeriod::OneTime => 0,
    }
}

/// Proration factor for a single component.
///
/// When the component bills on the same cadence as the subscription's current
/// period (the common case), we use the exact day-based factor so existing
/// behaviour is unchanged. When the component's cadence differs (e.g. a yearly
/// component added to a monthly subscription), we prorate against the
/// component's own nominal period length, so a yearly fee is charged for
/// `days_remaining / 365` rather than `days_remaining / days_in_month`.
pub(crate) fn component_proration_factor(
    period: &SubscriptionFeeBillingPeriod,
    days_remaining: i64,
    days_in_period: i64,
    base_factor: DayFraction,
) -> DayFraction {
    let nominal = nominal_period_days(period);
    if nominal <= 0 {
        return base_factor;
    }
    // Aligned with the subscription period when within 25% of the nominal length.
    if (days_in_period - nominal).abs() * 4 <= nominal {
        base_factor
    } else {
        DayFraction::new(days_remaining, nominal)
    }
}

/// Monthly share of an amount billed every `period`, for comparisons only.
fn per_month(amount: Decimal, period: &SubscriptionFeeBillingPeriod) -> Decimal {
    match period.as_months() {
        0 => Decimal::ZERO,
        months => amount / Decimal::from(months),
    }
}

/// What a credit pack adds per month to its denomination, compared per denomination.
#[derive(Default, Clone, Copy)]
struct PackTotals {
    fee: Decimal,
    credit: Decimal,
    /// `None` within `Some`: rolls over everything left.
    rollover: Option<Option<Decimal>>,
}

impl PackTotals {
    fn add(&mut self, fee: &SubscriptionFee, period: &SubscriptionFeeBillingPeriod) {
        let SubscriptionFee::Credit {
            rate,
            quantity,
            credit_amount,
            rollover,
            ..
        } = fee
        else {
            return;
        };
        let units = Decimal::from(*quantity);
        self.fee += per_month(*rate * units, period);
        if is_period_tied(fee) {
            self.credit += per_month(*credit_amount * units, period);
        }
        if let Some(rollover) = rollover {
            let cap = rollover
                .max_amount
                .map(|max| per_month(max * units, period));
            self.rollover = Some(match (self.rollover, cap) {
                (Some(None), _) | (_, None) => None,
                (Some(Some(total)), Some(cap)) => Some(total + cap),
                (None, Some(cap)) => Some(cap),
            });
        }
    }
}

fn rollover_cmp(new: Option<Option<Decimal>>, old: Option<Option<Decimal>>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (new, old) {
        (None, None) | (Some(None), Some(None)) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(None), Some(Some(_))) => Ordering::Greater,
        (Some(Some(_)), Some(None)) => Ordering::Less,
        (Some(Some(new)), Some(Some(old))) => new.cmp(&old),
    }
}

/// A pack whose credit ends with the billing period (with or without rollover): billed and
/// granted by the share of the period it covers. Any other pack is bought whole.
pub fn is_period_tied(fee: &SubscriptionFee) -> bool {
    matches!(
        fee,
        SubscriptionFee::Credit {
            expiry: Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            once: false,
            ..
        }
    )
}

fn pack_denomination(fee: &SubscriptionFee) -> Option<String> {
    match fee {
        SubscriptionFee::Credit {
            once: false,
            credit_unit,
            ..
        } => Some(
            credit_unit
                .as_ref()
                .map_or_else(String::new, |u| u.code.clone()),
        ),
        _ => None,
    }
}

/// Upgrade, downgrade or lateral, compared per month at the price the plan lists (the MRR basis):
/// the fixed fees in total, then per credit denomination the packs' fees, the credit of the packs
/// tied to the period and their rollover cap, and the billing period's length. Any decrease, a
/// credit denomination removed, or a shorter period is a downgrade, even beside an increase;
/// otherwise any increase is an upgrade. Usage prices are not compared.
pub fn detect_change_direction(
    matched: &[MatchedComponent],
    added: &[AddedComponent],
    removed: &[RemovedComponent],
    precision: u8,
) -> ChangeDirection {
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

    let fee = |fee: &SubscriptionFee, period: &SubscriptionFeeBillingPeriod| {
        per_month(
            Decimal::from(component_advance_amount_cents(fee, period, precision)),
            period,
        )
    };
    let mut outcomes: Vec<Ordering> = Vec::new();
    let mut old_packs: BTreeMap<String, PackTotals> = BTreeMap::new();
    let mut new_packs: BTreeMap<String, PackTotals> = BTreeMap::new();
    let mut old_period: Option<i32> = None;
    let mut new_period: Option<i32> = None;
    let shortest = |acc: &mut Option<i32>, period: &SubscriptionFeeBillingPeriod| {
        if period.as_months() > 0 {
            *acc = Some(acc.map_or(period.as_months(), |m| m.min(period.as_months())));
        }
    };

    let mut old_side = |f: &SubscriptionFee, p: &SubscriptionFeeBillingPeriod| {
        shortest(&mut old_period, p);
        if let Some(key) = pack_denomination(f) {
            old_packs.entry(key).or_default().add(f, p);
        }
    };
    for m in matched {
        old_side(&m.current_fee, &m.current_period);
    }
    for r in removed {
        old_side(&r.current_fee, &r.current_period);
    }
    let mut new_side = |f: &SubscriptionFee, p: &SubscriptionFeeBillingPeriod| {
        shortest(&mut new_period, p);
        if let Some(key) = pack_denomination(f) {
            new_packs.entry(key).or_default().add(f, p);
        }
    };
    for m in matched {
        new_side(&m.new_fee, &m.new_period);
    }
    for a in added {
        new_side(&a.fee, &a.period);
    }

    // Fixed fees compare in total, whatever products carry them: catalogs bill the base fee of
    // each plan under its own product.
    let fixed = |fee_and_period: &mut dyn Iterator<
        Item = (&SubscriptionFee, &SubscriptionFeeBillingPeriod),
    >| {
        fee_and_period
            .filter(|(f, _)| !matches!(f, SubscriptionFee::Credit { .. }))
            .map(|(f, p)| fee(f, p))
            .sum::<Decimal>()
    };
    let old_fixed = fixed(
        &mut matched
            .iter()
            .map(|m| (&m.current_fee, &m.current_period))
            .chain(removed.iter().map(|r| (&r.current_fee, &r.current_period))),
    );
    let new_fixed = fixed(
        &mut matched
            .iter()
            .map(|m| (&m.new_fee, &m.new_period))
            .chain(added.iter().map(|a| (&a.fee, &a.period))),
    );
    outcomes.push(new_fixed.cmp(&old_fixed));
    for (key, old) in &old_packs {
        match new_packs.get(key) {
            None => outcomes.push(Ordering::Less),
            Some(new) => {
                outcomes.push(new.fee.cmp(&old.fee));
                outcomes.push(new.credit.cmp(&old.credit));
                outcomes.push(rollover_cmp(new.rollover, old.rollover));
            }
        }
    }
    if new_packs.keys().any(|key| !old_packs.contains_key(key)) {
        outcomes.push(Ordering::Greater);
    }
    if let (Some(old), Some(new)) = (old_period, new_period)
        && new < old
    {
        outcomes.push(Ordering::Less);
    }

    if outcomes.contains(&Ordering::Less) {
        ChangeDirection::Downgrade
    } else if outcomes.contains(&Ordering::Greater) {
        ChangeDirection::Upgrade
    } else {
        ChangeDirection::Lateral
    }
}

/// The fee credited back for an old pack tied to the period: its unused time, but never more than
/// its unused paid credit is worth, so spending the credit and then changing plan refunds nothing.
pub fn capped_pack_fee_credit(
    fee_minor: i64,
    unused_time: DayFraction,
    unused_paid: i64,
    paid_face: i64,
) -> i64 {
    let by_time = unused_time.prorate(fee_minor);
    if paid_face <= 0 {
        return by_time;
    }
    by_time.min(DayFraction::new(unused_paid, paid_face).prorate(fee_minor))
}

/// The adjustment-invoice line of a pack the target plan adds: a pack tied to the period is
/// billed and granted by the share of the period left, any other pack whole.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pack_charge_line(
    name: &str,
    fee: &SubscriptionFee,
    factor: DayFraction,
    precision: u8,
    product_id: Option<common_domain::ids::ProductId>,
    price_component_id: Option<common_domain::ids::PriceComponentId>,
    billed_component_id: Option<common_domain::ids::SubscriptionPriceComponentId>,
) -> crate::StoreResult<Option<ProrationLineItem>> {
    use crate::services::invoice_lines::component::{pack_credit_minor, pack_rollover};
    let SubscriptionFee::Credit {
        rate,
        quantity,
        credit_amount,
        expiry,
        credit_unit,
        rollover,
        ..
    } = fee
    else {
        return Ok(None);
    };
    let factor = if is_period_tied(fee) {
        factor
    } else {
        DayFraction::WHOLE
    };
    let units = Decimal::from(*quantity);
    let credit_precision = credit_unit
        .as_ref()
        .map_or(precision, |unit| unit.precision);
    let credit_minor = factor.prorate(pack_credit_minor(*credit_amount, units, credit_precision)?);
    if credit_minor <= 0 {
        return Ok(None);
    }
    let full_fee = (*rate * units)
        .to_subunit_opt(precision)
        .ok_or_else(|| error_stack::Report::new(crate::errors::StoreError::InvalidDecimal))?;
    let included = rate.is_zero();
    let rollover = match rollover {
        Some(rollover) if included => Some(pack_rollover(rollover, units, credit_precision)?),
        _ => None,
    };
    let prorated = !factor.is_whole();
    Ok(Some(ProrationLineItem {
        period_end: None,
        name: if prorated {
            format!("{name} (prorated)")
        } else {
            name.to_owned()
        },
        amount_cents: factor.prorate(full_fee),
        full_period_amount_cents: full_fee,
        is_credit: false,
        is_prorated: prorated,
        quantity: Some(units),
        unit_price: None,
        product_id: crate::services::subscriptions::utils::fee_tax_product(fee, product_id),
        price_component_id,
        net_key: None,
        sub_component_id: billed_component_id,
        sub_add_on_id: None,
        charge_type: Some(fee.charge_type()),
        pack: Some(PackGrant {
            credit_minor,
            unit: credit_unit.clone(),
            expiry: *expiry,
            included,
            rollover,
        }),
    }))
}

/// Calculate proration for all advance-billed components.
///
/// factor = (period_end - change_date) / (period_end - period_start)
/// Credit per old component: -(old_amount * factor)
/// Charge per new component: +(new_amount * factor)
pub fn calculate_proration(
    matched: &[MatchedComponent],
    added: &[AddedComponent],
    removed: &[RemovedComponent],
    period_start: NaiveDate,
    period_end: NaiveDate,
    change_date: NaiveDate,
    precision: u8,
) -> ProrationResult {
    let days_in_period = (period_end - period_start).num_days();
    let days_remaining = (period_end - change_date).num_days();
    let proration_factor = DayFraction::remaining(period_start, period_end, change_date);
    calculate_proration_by(
        matched,
        added,
        removed,
        (period_start, period_end),
        change_date,
        precision,
        |period| {
            (
                component_proration_factor(
                    period,
                    days_remaining,
                    days_in_period,
                    proration_factor,
                ),
                None,
            )
        },
    )
}

/// The stub a fee of `period` bills from `change_date`: on the subscription's cadence, what is
/// left of `current`; on another, what is left of its own period on the grid from `billing_start`.
pub(crate) fn stub_on_own_grid(
    current: (NaiveDate, NaiveDate),
    cadence: crate::domain::enums::BillingPeriodEnum,
    billing_start: NaiveDate,
    billing_day: u32,
    change_date: NaiveDate,
) -> impl Fn(&SubscriptionFeeBillingPeriod) -> (DayFraction, Option<NaiveDate>) {
    move |period| {
        let (start, end) = match period.as_billing_period_opt() {
            Some(own) if own.as_months() != cadence.as_months() => {
                let own = crate::utils::periods::find_period_containing_date(
                    billing_start,
                    change_date,
                    &own,
                    billing_day,
                );
                (own.start, own.end)
            }
            _ => current,
        };
        (DayFraction::remaining(start, end, change_date), Some(end))
    }
}

/// [`calculate_proration`] with each fee's share and stub end given by `stub`.
pub(crate) fn calculate_proration_by(
    matched: &[MatchedComponent],
    added: &[AddedComponent],
    removed: &[RemovedComponent],
    (period_start, period_end): (NaiveDate, NaiveDate),
    change_date: NaiveDate,
    precision: u8,
    stub: impl Fn(&SubscriptionFeeBillingPeriod) -> (DayFraction, Option<NaiveDate>),
) -> ProrationResult {
    let proration_factor = DayFraction::remaining(period_start, period_end, change_date);
    let factor_of = |period: &SubscriptionFeeBillingPeriod| stub(period).0;
    let end_of = |period: &SubscriptionFeeBillingPeriod| stub(period).1;

    let mut lines = Vec::new();

    // Matched components: credit old, charge new
    for m in matched {
        let old_amount =
            component_advance_amount_cents(&m.current_fee, &m.current_period, precision);
        let new_amount = component_advance_amount_cents(&m.new_fee, &m.new_period, precision);

        if old_amount > 0 {
            let credit = -factor_of(&m.current_period).prorate(old_amount);
            if credit != 0 {
                lines.push(ProrationLineItem {
                    period_end: end_of(&m.current_period),
                    name: format!("{} (credit)", m.current_name),
                    amount_cents: credit,
                    full_period_amount_cents: old_amount,
                    is_credit: true,
                    is_prorated: true,
                    quantity: None,
                    unit_price: None,
                    product_id: Some(m.product_id),
                    price_component_id: None,
                    net_key: m.current_net_key.clone(),
                    sub_component_id: None,
                    sub_add_on_id: None,
                    charge_type: Some(m.current_fee.charge_type()),
                    pack: None,
                });
            }
        }

        if new_amount > 0 {
            let charge = factor_of(&m.new_period).prorate(new_amount);
            if charge != 0 {
                lines.push(ProrationLineItem {
                    period_end: end_of(&m.new_period),
                    name: format!("{} (prorated)", m.new_name),
                    amount_cents: charge,
                    full_period_amount_cents: new_amount,
                    is_credit: false,
                    is_prorated: true,
                    quantity: None,
                    unit_price: None,
                    product_id: Some(m.product_id),
                    price_component_id: m.price_component_id,
                    net_key: None,
                    sub_component_id: m.billed_component_id,
                    sub_add_on_id: None,
                    charge_type: Some(m.new_fee.charge_type()),
                    pack: None,
                });
            }
        }
    }

    // Removed components: credit
    for r in removed {
        let old_amount =
            component_advance_amount_cents(&r.current_fee, &r.current_period, precision);
        if old_amount > 0 {
            let credit = -factor_of(&r.current_period).prorate(old_amount);
            if credit != 0 {
                lines.push(ProrationLineItem {
                    period_end: end_of(&r.current_period),
                    name: format!("{} (credit)", r.name),
                    amount_cents: credit,
                    full_period_amount_cents: old_amount,
                    is_credit: true,
                    is_prorated: true,
                    quantity: None,
                    unit_price: None,
                    product_id: None,
                    price_component_id: None,
                    net_key: r.net_key.clone(),
                    sub_component_id: None,
                    sub_add_on_id: None,
                    charge_type: Some(r.current_fee.charge_type()),
                    pack: None,
                });
            }
        }
    }

    // Added components: charge. One-time fees are billed in full (never prorated);
    // recurring fees are prorated against their own billing cadence.
    for a in added {
        if let SubscriptionFee::OneTime { rate, quantity } = &a.fee {
            let amount = component_onetime_amount_cents(&a.fee, precision);
            if amount != 0 {
                lines.push(ProrationLineItem {
                    period_end: None,
                    name: format!("{} (one-time)", a.name),
                    amount_cents: amount,
                    full_period_amount_cents: amount,
                    is_credit: false,
                    is_prorated: false,
                    quantity: Some(rust_decimal::Decimal::from(*quantity)),
                    unit_price: Some(*rate),
                    product_id: None,
                    price_component_id: a.price_component_id,
                    net_key: a.net_key.clone(),
                    sub_component_id: a.billed_component_id,
                    sub_add_on_id: a.billed_add_on_id,
                    charge_type: Some(a.fee.charge_type()),
                    pack: None,
                });
            }
            continue;
        }

        let new_amount = component_advance_amount_cents(&a.fee, &a.period, precision);
        if new_amount > 0 {
            let charge = factor_of(&a.period).prorate(new_amount);
            if charge != 0 {
                // For multi-instance add-ons carry the instance count so the
                // adjustment invoice displays qty × unit_price. unit_price is
                // left as None so draft.rs derives it as amount / qty, keeping
                // qty × unit_price = total consistent on the invoice.
                let display_qty = a
                    .instance_quantity
                    .filter(|&n| n > rust_decimal::Decimal::ONE);
                lines.push(ProrationLineItem {
                    period_end: end_of(&a.period),
                    name: format!("{} (prorated)", a.name),
                    amount_cents: charge,
                    full_period_amount_cents: new_amount,
                    is_credit: false,
                    is_prorated: true,
                    quantity: display_qty,
                    unit_price: None,
                    product_id: None,
                    price_component_id: a.price_component_id,
                    net_key: a.net_key.clone(),
                    sub_component_id: a.billed_component_id,
                    sub_add_on_id: a.billed_add_on_id,
                    charge_type: Some(a.fee.charge_type()),
                    pack: None,
                });
            }
        }
    }

    let net_amount_cents: i64 = lines.iter().map(|l| l.amount_cents).sum();

    ProrationResult {
        lines,
        net_amount_cents,
        change_date,
        period_start,
        period_end,
        proration_factor: proration_factor.as_f64(),
    }
}

/// Net override credit/charge pairs that share a `net_key` into a single line,
/// so the adjustment invoice taxes the delta rather than a gross charge beside
/// an untaxed credit. Lines without a key (genuine adds/removes) pass through.
/// Zero-net lines are dropped.
pub fn net_override_lines(lines: &[ProrationLineItem]) -> Vec<ProrationLineItem> {
    let mut netted: Vec<ProrationLineItem> = Vec::new();
    let mut index_by_key: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for line in lines {
        match &line.net_key {
            Some(key) => {
                if let Some(&idx) = index_by_key.get(key) {
                    netted[idx].amount_cents += line.amount_cents;
                    // Prefer the charge (new price) line's identity for display.
                    if !line.is_credit {
                        netted[idx].name = line.name.clone();
                        netted[idx].is_prorated = line.is_prorated;
                        netted[idx].product_id = line.product_id;
                        netted[idx].price_component_id = line.price_component_id;
                        netted[idx].period_end = line.period_end;
                    }
                } else {
                    index_by_key.insert(key.clone(), netted.len());
                    netted.push(line.clone());
                }
            }
            None => netted.push(line.clone()),
        }
    }

    for nl in &mut netted {
        if nl.net_key.is_some() {
            let base = nl
                .name
                .rsplit_once(" (")
                .map(|(b, _)| b.to_string())
                .unwrap_or_else(|| nl.name.clone());
            nl.name = format!("{base} (adjustment)");
            nl.is_credit = nl.amount_cents < 0;
        }
    }

    // An included pack bills nothing but grants its credit.
    netted
        .into_iter()
        .filter(|l| l.amount_cents != 0 || l.pack.is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn rate_fee(amount: i64) -> SubscriptionFee {
        SubscriptionFee::Rate {
            rate: Decimal::new(amount, 0),
        }
    }

    fn usage_fee() -> SubscriptionFee {
        use common_domain::ids::BaseId;
        SubscriptionFee::Usage {
            metric_id: common_domain::ids::BillableMetricId::new(),
            model: crate::domain::UsagePricingModel::PerUnit { rate: Decimal::ONE },
            credit_unit: None,
        }
    }

    fn slot_fee(slots: u32, rate: i64) -> SubscriptionFee {
        SubscriptionFee::Slot {
            unit: "seat".to_string(),
            unit_rate: Decimal::new(rate, 0),
            min_slots: None,
            max_slots: None,
            initial_slots: slots,
        }
    }

    fn monthly() -> SubscriptionFeeBillingPeriod {
        SubscriptionFeeBillingPeriod::Monthly
    }

    fn product_id() -> common_domain::ids::ProductId {
        use common_domain::ids::BaseId;
        common_domain::ids::ProductId::new()
    }

    #[test]
    fn test_simple_upgrade_half_period() {
        // Rate 100 → 200, 15/30 days remaining
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Basic".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Pro".to_string(),
            new_fee: rate_fee(200),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &matched,
            &[],
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );

        assert_eq!(result.lines.len(), 2);
        // Credit: -(10000 * 15/30) = -5000
        assert_eq!(result.lines[0].amount_cents, -5000);
        assert!(result.lines[0].is_credit);
        // Charge: +(20000 * 15/30) = 10000
        assert_eq!(result.lines[1].amount_cents, 10000);
        assert!(!result.lines[1].is_credit);
        // Net: +5000
        assert_eq!(result.net_amount_cents, 5000);
    }

    #[test]
    fn test_downgrade_detection() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Pro".to_string(),
            current_fee: rate_fee(200),
            current_period: monthly(),
            new_name: "Basic".to_string(),
            new_fee: rate_fee(100),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&matched, &[], &[], 2);
        assert_eq!(direction, ChangeDirection::Downgrade);
    }

    #[test]
    fn test_upgrade_detection() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Basic".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Pro".to_string(),
            new_fee: rate_fee(200),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&matched, &[], &[], 2);
        assert_eq!(direction, ChangeDirection::Upgrade);
    }

    #[test]
    fn test_lateral_detection() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Plan A".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Plan B".to_string(),
            new_fee: rate_fee(100),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&matched, &[], &[], 2);
        assert_eq!(direction, ChangeDirection::Lateral);
    }

    #[test]
    fn test_usage_excluded_from_proration() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "API Calls".to_string(),
            current_fee: usage_fee(),
            current_period: monthly(),
            new_name: "API Calls".to_string(),
            new_fee: usage_fee(),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &matched,
            &[],
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );

        assert!(result.lines.is_empty());
        assert_eq!(result.net_amount_cents, 0);
    }

    #[test]
    fn test_change_on_period_start() {
        // Factor = 1.0 (full period remaining)
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Basic".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Pro".to_string(),
            new_fee: rate_fee(200),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &matched,
            &[],
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            2,
        );

        // Factor = 30/30 = 1.0
        assert_eq!(result.lines[0].amount_cents, -10000); // full credit
        assert_eq!(result.lines[1].amount_cents, 20000); // full charge
        assert_eq!(result.net_amount_cents, 10000);
    }

    #[test]
    fn test_change_on_period_end() {
        // Factor = 0.0 (no days remaining)
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Basic".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Pro".to_string(),
            new_fee: rate_fee(200),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &matched,
            &[],
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            2,
        );

        assert!(result.lines.is_empty());
        assert_eq!(result.net_amount_cents, 0);
    }

    #[test]
    fn test_change_on_last_day() {
        // Factor = 1/30
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Basic".to_string(),
            current_fee: rate_fee(300),
            current_period: monthly(),
            new_name: "Pro".to_string(),
            new_fee: rate_fee(600),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &matched,
            &[],
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 30).unwrap(),
            2,
        );

        // Factor = 1/30
        // Credit: -(30000 * 1/30) = -1000
        assert_eq!(result.lines[0].amount_cents, -1000);
        // Charge: +(60000 * 1/30) = 2000
        assert_eq!(result.lines[1].amount_cents, 2000);
        assert_eq!(result.net_amount_cents, 1000);
    }

    #[test]
    fn test_mixed_components_with_added_and_removed() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Base".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            new_name: "Base Pro".to_string(),
            new_fee: rate_fee(200),
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let added = vec![AddedComponent {
            name: "Feature X".to_string(),
            fee: rate_fee(50),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];

        let removed = vec![RemovedComponent {
            name: "Feature Y".to_string(),
            current_fee: rate_fee(30),
            current_period: monthly(),
            net_key: None,
        }];

        let result = calculate_proration(
            &matched,
            &added,
            &removed,
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );

        // Factor = 15/30 = 0.5
        // Matched credit: -(10000 * 0.5) = -5000
        // Matched charge: +(20000 * 0.5) = 10000
        // Removed credit: -(3000 * 0.5) = -1500
        // Added charge: +(5000 * 0.5) = 2500
        assert_eq!(result.lines.len(), 4);
        let net: i64 = result.lines.iter().map(|l| l.amount_cents).sum();
        assert_eq!(net, -5000 + 10000 - 1500 + 2500);
        assert_eq!(result.net_amount_cents, 6000);
    }

    #[test]
    fn test_slot_component_proration() {
        let pid = product_id();
        let matched = vec![MatchedComponent {
            product_id: pid,
            current_name: "Seats".to_string(),
            current_fee: slot_fee(5, 10), // 5 seats * $10 = $50
            current_period: monthly(),
            new_name: "Seats".to_string(),
            new_fee: slot_fee(10, 10), // 10 seats * $10 = $100
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &matched,
            &[],
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );

        // Factor = 15/30 = 0.5
        // Credit: -(5000 * 0.5) = -2500
        // Charge: +(10000 * 0.5) = 5000
        assert_eq!(result.lines[0].amount_cents, -2500);
        assert_eq!(result.lines[1].amount_cents, 5000);
        assert_eq!(result.net_amount_cents, 2500);
    }

    #[test]
    fn test_direction_with_added_components() {
        let added = vec![AddedComponent {
            name: "New Feature".to_string(),
            fee: rate_fee(100),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];

        let direction = detect_change_direction(&[], &added, &[], 2);
        assert_eq!(direction, ChangeDirection::Upgrade);
    }

    #[test]
    fn test_direction_with_removed_components() {
        let removed = vec![RemovedComponent {
            name: "Old Feature".to_string(),
            current_fee: rate_fee(100),
            current_period: monthly(),
            net_key: None,
        }];

        let direction = detect_change_direction(&[], &[], &removed, 2);
        assert_eq!(direction, ChangeDirection::Downgrade);
    }

    #[test]
    fn test_added_one_time_charged_in_full() {
        // A one-time fee added mid-period is billed in full, never prorated.
        let added = vec![AddedComponent {
            name: "Setup".to_string(),
            fee: SubscriptionFee::OneTime {
                rate: Decimal::new(500, 0),
                quantity: 2,
            },
            period: SubscriptionFeeBillingPeriod::OneTime,
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];

        // Only 8 of 30 days remain — must not scale the one-time charge.
        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 23).unwrap(),
            2,
        );

        assert_eq!(result.lines.len(), 1);
        assert!(!result.lines[0].is_credit);
        // 500 * 2 = 1000 → 100000 cents, in full.
        assert_eq!(result.lines[0].amount_cents, 100_000);
        assert_eq!(result.net_amount_cents, 100_000);
    }

    #[test]
    fn test_added_yearly_component_prorated_over_its_own_period() {
        // A yearly component added to a 30-day (monthly) period must be prorated
        // over the year (days_remaining / 365), not over the month.
        let added = vec![AddedComponent {
            name: "Annual".to_string(),
            fee: rate_fee(3650),
            period: SubscriptionFeeBillingPeriod::Annual,
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            2,
        );

        assert_eq!(result.lines.len(), 1);
        // 365000 cents * 30/365 = 30000, NOT 365000 * 30/30.
        assert_eq!(result.lines[0].amount_cents, 30_000);
    }

    #[test]
    fn test_added_aligned_monthly_uses_exact_period_factor() {
        // A monthly component on a 30-day period keeps the exact day-based factor.
        let added = vec![AddedComponent {
            name: "Monthly".to_string(),
            fee: rate_fee(100),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];

        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );

        // 10000 * 15/30 = 5000.
        assert_eq!(result.lines[0].amount_cents, 5000);
    }

    // A genuinely-added add-on stamps its pre-generated subscription-add-on id onto
    // the prorated charge line, and `net_override_lines` preserves it. This is what
    // lets a later removal match and credit the add-on's adjustment-invoice line.
    #[test]
    fn test_genuine_add_tags_line_with_billed_id() {
        use common_domain::ids::{BaseId, SubscriptionAddOnId, SubscriptionPriceComponentId};

        let aid = SubscriptionAddOnId::new();
        let added = vec![AddedComponent {
            name: "Support".to_string(),
            fee: rate_fee(2000),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: Some(aid),
            instance_quantity: None,
            price_component_id: None,
        }];
        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].sub_add_on_id, Some(aid));
        assert_eq!(result.lines[0].sub_component_id, None);

        // Pass-through netting (no opposite-keyed line) keeps the stamp.
        let netted = net_override_lines(&result.lines);
        assert_eq!(netted.len(), 1);
        assert_eq!(netted[0].sub_add_on_id, Some(aid));

        // A component add tags sub_component_id instead.
        let cid = SubscriptionPriceComponentId::new();
        let added = vec![AddedComponent {
            name: "Extra".to_string(),
            fee: rate_fee(100),
            period: monthly(),
            net_key: None,
            billed_component_id: Some(cid),
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }];
        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );
        assert_eq!(result.lines[0].sub_component_id, Some(cid));
        assert_eq!(result.lines[0].sub_add_on_id, None);
    }

    // Multi-instance add-on: 3 × $100/mo, half-period (15/30).
    // Amount must be 3 × $100 × 0.5 = $150 = 15000 cents.
    // Display: qty=3, unit_price = $100 (full-period per-unit rate, not prorated).
    #[test]
    fn test_multi_instance_addon_prorated_display() {
        use rust_decimal_macros::dec;

        let added = vec![AddedComponent {
            name: "Token".to_string(),
            fee: rate_fee(300), // scaled: 3 × $100
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: Some(dec!(3)),
            price_component_id: None,
        }];

        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(), // 15/30 remaining
            2,
        );

        assert_eq!(result.lines.len(), 1);
        let line = &result.lines[0];
        // Amount: 30000 cents × 0.5 = 15000
        assert_eq!(line.amount_cents, 15000);
        // Display qty = 3; unit_price left None so draft.rs derives 150/3 = $50
        assert_eq!(line.quantity, Some(dec!(3)));
        assert_eq!(line.unit_price, None);
    }

    // Single-instance add-on: no display decomposition (avoids showing qty=1 noise).
    #[test]
    fn test_single_instance_addon_no_display_qty() {
        let added = vec![AddedComponent {
            name: "Base".to_string(),
            fee: rate_fee(100),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: Some(rust_decimal::Decimal::ONE),
            price_component_id: None,
        }];

        let result = calculate_proration(
            &[],
            &added,
            &[],
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            NaiveDate::from_ymd_opt(2025, 1, 16).unwrap(),
            2,
        );

        assert_eq!(result.lines.len(), 1);
        let line = &result.lines[0];
        assert_eq!(line.amount_cents, 5000); // 10000 × 0.5
        assert_eq!(line.quantity, None);
        assert_eq!(line.unit_price, None);
    }

    fn line(name: &str, amount: i64, is_credit: bool, net_key: Option<&str>) -> ProrationLineItem {
        ProrationLineItem {
            period_end: None,
            name: name.to_string(),
            amount_cents: amount,
            full_period_amount_cents: amount.abs(),
            is_credit,
            is_prorated: true,
            quantity: None,
            unit_price: None,
            product_id: None,
            price_component_id: None,
            net_key: net_key.map(str::to_string),
            sub_component_id: None,
            sub_add_on_id: None,
            charge_type: None,
            pack: None,
        }
    }

    #[test]
    fn test_net_override_price_drop_nets_to_single_credit() {
        // Override 100 -> 50: old credit -100, new charge +50, same key → net -50.
        let netted = net_override_lines(&[
            line("Base (credit)", -10000, true, Some("c1")),
            line("Base (prorated)", 5000, false, Some("c1")),
        ]);
        assert_eq!(netted.len(), 1);
        assert_eq!(netted[0].amount_cents, -5000);
        assert!(netted[0].is_credit);
        assert_eq!(netted[0].name, "Base (adjustment)");
    }

    #[test]
    fn test_net_override_price_increase_nets_to_single_charge() {
        // Override 50 -> 100: credit -50, charge +100 → net +50 (taxed on the delta).
        let netted = net_override_lines(&[
            line("Base (credit)", -5000, true, Some("c1")),
            line("Base (prorated)", 10000, false, Some("c1")),
        ]);
        assert_eq!(netted.len(), 1);
        assert_eq!(netted[0].amount_cents, 5000);
        assert!(!netted[0].is_credit);
    }

    #[test]
    fn test_net_override_keeps_unkeyed_lines_separate() {
        // Genuine add + genuine remove (no key) are not netted together.
        let netted = net_override_lines(&[
            line("Add A (prorated)", 3000, false, None),
            line("Remove B (credit)", -2000, true, None),
        ]);
        assert_eq!(netted.len(), 2);
    }

    #[test]
    fn test_net_override_zero_net_is_dropped() {
        let netted = net_override_lines(&[
            line("Base (credit)", -5000, true, Some("c1")),
            line("Base (prorated)", 5000, false, Some("c1")),
        ]);
        assert!(netted.is_empty());
    }

    #[test]
    fn fee_instance_count_reads_count_from_fee() {
        use crate::domain::enums::BillingType;
        use rust_decimal_macros::dec;

        // Count encoded in the fee.
        assert_eq!(fee_instance_count(&slot_fee(3, 10)), dec!(3));
        assert_eq!(
            fee_instance_count(&SubscriptionFee::Recurring {
                rate: dec!(10),
                quantity: 5,
                billing_type: BillingType::Advance,
            }),
            dec!(5)
        );
        assert_eq!(
            fee_instance_count(&SubscriptionFee::OneTime {
                rate: dec!(10),
                quantity: 2,
            }),
            dec!(2)
        );

        // Flat / countless fees are a single unit (shared match arm: Rate | Capacity | Usage).
        assert_eq!(fee_instance_count(&rate_fee(100)), dec!(1));
        assert_eq!(fee_instance_count(&usage_fee()), dec!(1));
    }

    fn pack(
        rate: i64,
        credit: i64,
        expiry: Option<crate::domain::price_components::CreditExpiry>,
        unit: Option<&str>,
        rollover_cap: Option<i64>,
    ) -> SubscriptionFee {
        use crate::domain::price_components::{CreditExpiry, CreditRollover};
        SubscriptionFee::Credit {
            rate: Decimal::new(rate, 0),
            quantity: 1,
            credit_amount: Decimal::new(credit, 0),
            expiry,
            credit_unit: unit.map(|code| crate::domain::credit_units::CreditUnitRef {
                unit_id: {
                    use common_domain::ids::BaseId;
                    common_domain::ids::CreditUnitId::new()
                },
                code: code.to_owned(),
                precision: 2,
                product_id: None,
            }),
            rollover: rollover_cap.map(|cap| CreditRollover {
                max_amount: Some(Decimal::new(cap, 0)),
                expiry: CreditExpiry::Month(1),
            }),
            once: false,
        }
    }

    fn period_pack(rate: i64, credit: i64) -> SubscriptionFee {
        pack(
            rate,
            credit,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            None,
            None,
        )
    }

    fn matched(current: SubscriptionFee, new: SubscriptionFee) -> MatchedComponent {
        MatchedComponent {
            product_id: product_id(),
            current_name: "Current".to_string(),
            current_fee: current,
            current_period: monthly(),
            new_name: "New".to_string(),
            new_fee: new,
            new_period: monthly(),
            current_net_key: None,
            billed_component_id: None,
            price_component_id: None,
        }
    }

    fn added(fee: SubscriptionFee) -> AddedComponent {
        AddedComponent {
            name: "Added".to_string(),
            fee,
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
            price_component_id: None,
        }
    }

    fn removed(fee: SubscriptionFee) -> RemovedComponent {
        RemovedComponent {
            name: "Removed".to_string(),
            current_fee: fee,
            current_period: monthly(),
            net_key: None,
        }
    }

    #[test]
    fn a_day_fraction_rounds_the_exact_ratio_half_away_from_zero() {
        let f = DayFraction::remaining(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 16).unwrap(),
        );
        // 50000 × 16 / 31 = 25806.45…; 55000 × 16 / 31 = 28387.09…
        assert_eq!(f.prorate(50_000), 25_806);
        assert_eq!(f.prorate(55_000), 28_387);
        assert_eq!(f.prorate(-50_000), -25_806);
        // 15 of 30 days of 1 cent is exactly half a cent: away from zero, both ways.
        let half = DayFraction::new(15, 30);
        assert_eq!(half.prorate(1), 1);
        assert_eq!(half.prorate(-1), -1);
        assert_eq!(half.prorate(3), 2);
        assert_eq!(DayFraction::new(40, 30).prorate(3_000), 3_000);
        assert_eq!(DayFraction::new(5, 0).prorate(1_000), 0);
    }

    #[test]
    fn a_pack_fee_credit_is_capped_by_the_unused_paid_credit() {
        let half = DayFraction::new(15, 30);
        // Nothing used: the unused time decides.
        assert_eq!(capped_pack_fee_credit(50_000, half, 50_000, 50_000), 25_000);
        // A fifth left: a fifth of the fee, though half the period is left.
        assert_eq!(capped_pack_fee_credit(50_000, half, 10_000, 50_000), 10_000);
        // Everything spent, then the change: nothing back.
        assert_eq!(capped_pack_fee_credit(50_000, half, 0, 50_000), 0);
        // Never a grant (unpaid): only time.
        assert_eq!(capped_pack_fee_credit(50_000, half, 0, 0), 25_000);
        // Rounding of the cap: 50000 × 1 / 3 = 16666.67.
        assert_eq!(
            capped_pack_fee_credit(50_000, DayFraction::WHOLE, 1, 3),
            16_667
        );
    }

    #[test]
    fn a_period_pack_is_billed_and_granted_by_the_share_left_and_any_other_whole() {
        let f = DayFraction::new(16, 31);
        let line = pack_charge_line(
            "Monthly credit",
            &period_pack(500, 550),
            f,
            2,
            None,
            None,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(line.amount_cents, 25_806);
        assert_eq!(line.pack.as_ref().unwrap().credit_minor, 28_387);
        assert!(line.is_prorated);

        let yearly = pack(
            500,
            550,
            Some(crate::domain::price_components::CreditExpiry::Month(12)),
            None,
            None,
        );
        let line = pack_charge_line("Annual credit", &yearly, f, 2, None, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(line.amount_cents, 50_000);
        assert_eq!(line.pack.as_ref().unwrap().credit_minor, 55_000);
        assert!(!line.is_prorated);

        let included = pack(
            0,
            5_000,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            Some("CREDITS"),
            Some(1_000),
        );
        let line = pack_charge_line("Monthly credits", &included, f, 2, None, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(line.amount_cents, 0);
        let grant = line.pack.unwrap();
        assert!(grant.included);
        assert_eq!(grant.credit_minor, 258_065);
        assert_eq!(
            grant.rollover.unwrap().max_minor,
            Some(100_000),
            "the rollover cap is a ceiling, kept whole"
        );
    }

    #[test]
    fn direction_compares_packs_per_denomination() {
        let studio_credits = pack(
            0,
            5_000,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            Some("CREDITS"),
            Some(1_000),
        );
        // Scale -> Studio: the EUR pack goes, Studio's fees are lower.
        assert_eq!(
            detect_change_direction(
                &[matched(rate_fee(124), rate_fee(39))],
                &[added(studio_credits.clone())],
                &[removed(period_pack(500, 550))],
                2
            ),
            ChangeDirection::Downgrade
        );
        // Studio -> Scale: higher fees and a EUR pack, but the CREDITS unit is removed.
        assert_eq!(
            detect_change_direction(
                &[matched(rate_fee(39), rate_fee(124))],
                &[added(period_pack(500, 550))],
                &[removed(studio_credits.clone())],
                2
            ),
            ChangeDirection::Downgrade
        );
        // Free -> Scale: only additions.
        assert_eq!(
            detect_change_direction(&[], &[added(period_pack(500, 550))], &[], 2),
            ChangeDirection::Upgrade
        );
        // Scale -> a bigger EUR pack under another product: compared per denomination.
        assert_eq!(
            detect_change_direction(
                &[],
                &[added(period_pack(1_000, 1_100))],
                &[removed(period_pack(500, 550))],
                2
            ),
            ChangeDirection::Upgrade
        );
        // Same pack fee, less credit.
        assert_eq!(
            detect_change_direction(
                &[matched(period_pack(500, 550), period_pack(500, 500))],
                &[],
                &[],
                2
            ),
            ChangeDirection::Downgrade
        );
        // A lower rollover cap alone is a downgrade.
        let lower_cap = pack(
            0,
            5_000,
            Some(crate::domain::price_components::CreditExpiry::BillingPeriod),
            Some("CREDITS"),
            Some(500),
        );
        assert_eq!(
            detect_change_direction(&[matched(studio_credits.clone(), lower_cap)], &[], &[], 2),
            ChangeDirection::Downgrade
        );
        // Mixed: fixed fees up in total, the pack's credit down.
        assert_eq!(
            detect_change_direction(
                &[
                    matched(rate_fee(100), rate_fee(200)),
                    matched(period_pack(500, 550), period_pack(500, 500))
                ],
                &[],
                &[],
                2
            ),
            ChangeDirection::Downgrade
        );
        // A pack removed, nothing else changes.
        assert_eq!(
            detect_change_direction(
                &[matched(rate_fee(100), rate_fee(100))],
                &[],
                &[removed(studio_credits)],
                2
            ),
            ChangeDirection::Downgrade
        );
        // The same plan: lateral.
        assert_eq!(
            detect_change_direction(
                &[matched(period_pack(500, 550), period_pack(500, 550))],
                &[],
                &[],
                2
            ),
            ChangeDirection::Lateral
        );
    }

    #[test]
    fn fixed_fees_compare_in_total_across_products() {
        // Starter's 29 base fee and Pro's 99 under different products: an upgrade.
        assert_eq!(
            detect_change_direction(&[], &[added(rate_fee(99))], &[removed(rate_fee(29))], 2),
            ChangeDirection::Upgrade
        );
        // A recurring fee and a slot fee replaced by a single higher rate: still an upgrade.
        assert_eq!(
            detect_change_direction(
                &[],
                &[added(rate_fee(150))],
                &[removed(rate_fee(29)), removed(slot_fee(5, 10))],
                2
            ),
            ChangeDirection::Upgrade
        );
        // Lower in total: a downgrade.
        assert_eq!(
            detect_change_direction(&[], &[added(slot_fee(2, 10))], &[removed(rate_fee(29))], 2),
            ChangeDirection::Downgrade
        );
        // Same total: lateral.
        assert_eq!(
            detect_change_direction(&[], &[added(rate_fee(29))], &[removed(rate_fee(29))], 2),
            ChangeDirection::Lateral
        );
    }

    #[test]
    fn a_shorter_period_is_a_downgrade_even_at_the_same_monthly_price() {
        let mut m = matched(rate_fee(1_200), rate_fee(100));
        m.current_period = SubscriptionFeeBillingPeriod::Annual;
        assert_eq!(
            detect_change_direction(&[m], &[], &[], 2),
            ChangeDirection::Downgrade
        );
    }
}
