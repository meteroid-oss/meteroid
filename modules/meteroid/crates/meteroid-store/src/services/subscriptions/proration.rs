use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::subscription_changes::{
    AddedComponent, ChangeDirection, MatchedComponent, ProrationLineItem, ProrationResult,
    RemovedComponent,
};
use crate::domain::subscription_components::SubscriptionFee;
use chrono::NaiveDate;
use common_utils::decimals::ToSubunit;

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
        SubscriptionFee::OneTime { .. } | SubscriptionFee::Usage { .. } => 0,
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

/// Nominal length, in days, of a billing period. Used to prorate components
/// whose billing cadence differs from the subscription's current period.
fn nominal_period_days(period: &SubscriptionFeeBillingPeriod) -> f64 {
    match period {
        SubscriptionFeeBillingPeriod::Monthly => 30.0,
        SubscriptionFeeBillingPeriod::Quarterly => 91.0,
        SubscriptionFeeBillingPeriod::Semiannual => 182.0,
        SubscriptionFeeBillingPeriod::Annual => 365.0,
        SubscriptionFeeBillingPeriod::OneTime => 0.0,
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
fn component_proration_factor(
    period: &SubscriptionFeeBillingPeriod,
    days_remaining: f64,
    days_in_period: f64,
    base_factor: f64,
) -> f64 {
    let nominal = nominal_period_days(period);
    if nominal <= 0.0 {
        return base_factor;
    }
    // Treat the component as aligned with the subscription period when the
    // current period length is within 25% of the component's nominal length.
    if (days_in_period - nominal).abs() <= nominal * 0.25 {
        base_factor
    } else {
        (days_remaining / nominal).clamp(0.0, 1.0)
    }
}

/// Detect upgrade/downgrade by comparing total advance-billed amounts.
pub fn detect_change_direction(
    matched: &[MatchedComponent],
    added: &[AddedComponent],
    removed: &[RemovedComponent],
    precision: u8,
) -> ChangeDirection {
    let old_total: i64 =
        matched
            .iter()
            .map(|m| component_advance_amount_cents(&m.current_fee, &m.current_period, precision))
            .chain(removed.iter().map(|r| {
                component_advance_amount_cents(&r.current_fee, &r.current_period, precision)
            }))
            .sum();

    let new_total: i64 = matched
        .iter()
        .map(|m| component_advance_amount_cents(&m.new_fee, &m.new_period, precision))
        .chain(
            added
                .iter()
                .map(|a| component_advance_amount_cents(&a.fee, &a.period, precision)),
        )
        .sum();

    match new_total.cmp(&old_total) {
        std::cmp::Ordering::Greater => ChangeDirection::Upgrade,
        std::cmp::Ordering::Less => ChangeDirection::Downgrade,
        std::cmp::Ordering::Equal => ChangeDirection::Lateral,
    }
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
    let days_in_period = (period_end - period_start).num_days() as f64;
    let days_remaining = (period_end - change_date).num_days() as f64;

    let proration_factor = if days_in_period > 0.0 {
        days_remaining / days_in_period
    } else {
        0.0
    };

    let mut lines = Vec::new();

    // Matched components: credit old, charge new
    for m in matched {
        let old_amount =
            component_advance_amount_cents(&m.current_fee, &m.current_period, precision);
        let new_amount = component_advance_amount_cents(&m.new_fee, &m.new_period, precision);

        if old_amount > 0 {
            let factor = component_proration_factor(
                &m.current_period,
                days_remaining,
                days_in_period,
                proration_factor,
            );
            let credit = -((old_amount as f64 * factor).round() as i64);
            if credit != 0 {
                lines.push(ProrationLineItem {
                    name: format!("{} (credit)", m.current_name),
                    amount_cents: credit,
                    full_period_amount_cents: old_amount,
                    is_credit: true,
                    is_prorated: true,
                    quantity: None,
                    unit_price: None,
                    product_id: Some(m.product_id),
                    price_component_id: None,
                    net_key: None,
                    sub_component_id: None,
                    sub_add_on_id: None,
                });
            }
        }

        if new_amount > 0 {
            let factor = component_proration_factor(
                &m.new_period,
                days_remaining,
                days_in_period,
                proration_factor,
            );
            let charge = (new_amount as f64 * factor).round() as i64;
            if charge != 0 {
                lines.push(ProrationLineItem {
                    name: format!("{} (prorated)", m.new_name),
                    amount_cents: charge,
                    full_period_amount_cents: new_amount,
                    is_credit: false,
                    is_prorated: true,
                    quantity: None,
                    unit_price: None,
                    product_id: Some(m.product_id),
                    price_component_id: None,
                    net_key: None,
                    sub_component_id: None,
                    sub_add_on_id: None,
                });
            }
        }
    }

    // Removed components: credit
    for r in removed {
        let old_amount =
            component_advance_amount_cents(&r.current_fee, &r.current_period, precision);
        if old_amount > 0 {
            let factor = component_proration_factor(
                &r.current_period,
                days_remaining,
                days_in_period,
                proration_factor,
            );
            let credit = -((old_amount as f64 * factor).round() as i64);
            if credit != 0 {
                lines.push(ProrationLineItem {
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
                    name: format!("{} (one-time)", a.name),
                    amount_cents: amount,
                    full_period_amount_cents: amount,
                    is_credit: false,
                    is_prorated: false,
                    quantity: Some(rust_decimal::Decimal::from(*quantity)),
                    unit_price: Some(*rate),
                    product_id: None,
                    price_component_id: None,
                    net_key: a.net_key.clone(),
                    sub_component_id: a.billed_component_id,
                    sub_add_on_id: a.billed_add_on_id,
                });
            }
            continue;
        }

        let new_amount = component_advance_amount_cents(&a.fee, &a.period, precision);
        if new_amount > 0 {
            let factor = component_proration_factor(
                &a.period,
                days_remaining,
                days_in_period,
                proration_factor,
            );
            let charge = (new_amount as f64 * factor).round() as i64;
            if charge != 0 {
                // For multi-instance add-ons carry the instance count so the
                // adjustment invoice displays qty × unit_price. unit_price is
                // left as None so draft.rs derives it as amount / qty, keeping
                // qty × unit_price = total consistent on the invoice.
                let display_qty = a
                    .instance_quantity
                    .filter(|&n| n > rust_decimal::Decimal::ONE);
                lines.push(ProrationLineItem {
                    name: format!("{} (prorated)", a.name),
                    amount_cents: charge,
                    full_period_amount_cents: new_amount,
                    is_credit: false,
                    is_prorated: true,
                    quantity: display_qty,
                    unit_price: None,
                    product_id: None,
                    price_component_id: None,
                    net_key: a.net_key.clone(),
                    sub_component_id: a.billed_component_id,
                    sub_add_on_id: a.billed_add_on_id,
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
        proration_factor,
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

    netted.into_iter().filter(|l| l.amount_cents != 0).collect()
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
        }];

        let added = vec![AddedComponent {
            name: "Feature X".to_string(),
            fee: rate_fee(50),
            period: monthly(),
            net_key: None,
            billed_component_id: None,
            billed_add_on_id: None,
            instance_quantity: None,
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
}
