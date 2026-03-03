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
            let credit = -((old_amount as f64 * proration_factor).round() as i64);
            if credit != 0 {
                lines.push(ProrationLineItem {
                    name: format!("{} (credit)", m.current_name),
                    amount_cents: credit,
                    full_period_amount_cents: old_amount,
                    is_credit: true,
                    product_id: Some(m.product_id),
                    price_component_id: None,
                });
            }
        }

        if new_amount > 0 {
            let charge = (new_amount as f64 * proration_factor).round() as i64;
            if charge != 0 {
                lines.push(ProrationLineItem {
                    name: format!("{} (prorated)", m.new_name),
                    amount_cents: charge,
                    full_period_amount_cents: new_amount,
                    is_credit: false,
                    product_id: Some(m.product_id),
                    price_component_id: None,
                });
            }
        }
    }

    // Removed components: credit
    for r in removed {
        let old_amount =
            component_advance_amount_cents(&r.current_fee, &r.current_period, precision);
        if old_amount > 0 {
            let credit = -((old_amount as f64 * proration_factor).round() as i64);
            if credit != 0 {
                lines.push(ProrationLineItem {
                    name: format!("{} (credit)", r.name),
                    amount_cents: credit,
                    full_period_amount_cents: old_amount,
                    is_credit: true,
                    product_id: None,
                    price_component_id: None,
                });
            }
        }
    }

    // Added components: charge
    for a in added {
        let new_amount = component_advance_amount_cents(&a.fee, &a.period, precision);
        if new_amount > 0 {
            let charge = (new_amount as f64 * proration_factor).round() as i64;
            if charge != 0 {
                lines.push(ProrationLineItem {
                    name: format!("{} (prorated)", a.name),
                    amount_cents: charge,
                    full_period_amount_cents: new_amount,
                    is_credit: false,
                    product_id: None,
                    price_component_id: None,
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
        }];

        let removed = vec![RemovedComponent {
            name: "Feature Y".to_string(),
            current_fee: rate_fee(30),
            current_period: monthly(),
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
        }];

        let direction = detect_change_direction(&[], &[], &removed, 2);
        assert_eq!(direction, ChangeDirection::Downgrade);
    }
}
