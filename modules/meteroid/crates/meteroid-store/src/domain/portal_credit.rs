//! What the customer portal shows of a customer's credit, limits and subscription prices.
use crate::domain::BillableMetric;
use crate::domain::coupons::CouponDiscount;
use crate::domain::credit::{
    AutomaticTopUp, CreditGrant, CreditWallet, PortalCreditCapabilities, WalletHealth,
};
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::subscription_components::SubscriptionComponent;
use crate::domain::subscription_coupons::AppliedCouponDetailed;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::ids::{BillableMetricId, FeatureId, SubscriptionId};
use common_utils::decimals::ToSubunit;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};

/// Where a grant came from, as the customer reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalCreditSource {
    Purchased,
    Included,
    Added,
    RolledOver,
    AutomaticTopUp,
    CreditNote,
    Returned,
    PreviousBalance,
    Bonus,
    Overpayment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalCreditGrant {
    pub grant: CreditGrant,
    pub source: PortalCreditSource,
    /// Left once the usage not yet invoiced is paid; None until the wallet's currency is estimated.
    pub remaining_after_usage_minor: Option<i64>,
    /// Some when what is left at expiry rolls over: up to the cap, or all of it when None.
    pub rollover_max_minor: Option<Option<i64>>,
    pub invoice_number: Option<String>,
}

/// Since the wallet's first credit in effect. `added - used - expired` is the balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortalCreditTotals {
    pub since: Option<DateTime<Utc>>,
    pub added_minor: i64,
    /// Applied to invoices, plus the usage not yet invoiced.
    pub used_minor: i64,
    pub expired_minor: i64,
}

/// The next credit a subscription's recurring credit pack funds the wallet with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalNextCredit {
    pub amount_minor: i64,
    pub date: NaiveDate,
    /// Zero when the plan includes it at no charge.
    pub price_minor: i64,
    pub price_currency: String,
    pub cadence: SubscriptionFeeBillingPeriod,
    pub price_excludes_tax: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalUsageQuantity {
    pub quantity: Decimal,
    pub metric_name: String,
    pub unit_singular: Option<String>,
    pub unit_plural: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PortalWalletOverview {
    pub wallet: CreditWallet,
    pub auto_top_up: Option<AutomaticTopUp>,
    pub health: WalletHealth,
    /// Active and scheduled grants, in the order credit is used.
    pub grants: Vec<PortalCreditGrant>,
    pub period: Option<(NaiveDate, NaiveDate)>,
    pub projected_usage_minor: Option<i64>,
    pub usage_quantity: Option<PortalUsageQuantity>,
    pub totals: PortalCreditTotals,
    pub next_credit: Option<PortalNextCredit>,
    pub next_expiry: Option<PortalExpiringCredit>,
    pub usage_paused_at: Option<DateTime<Utc>>,
    /// Usage priced in the wallet's denomination over the drawing subscriptions' current periods.
    pub period_usage_minor: Option<i64>,
}

/// The soonest expiry: what is left on the grants expiring then, and how much of it rolls over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalExpiringCredit {
    pub expires_at: DateTime<Utc>,
    pub remaining_minor: i64,
    pub rollover_minor: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct PortalCreditOverview {
    pub wallets: Vec<PortalWalletOverview>,
    pub capabilities: PortalCreditCapabilities,
    /// A subscription funds usage from credit: a recurring credit pack, or credit usage stops at.
    pub credit_funded: bool,
    /// The next invoice of each active subscription, computed once for the whole overview.
    pub upcoming: Vec<PortalUpcomingInvoice>,
    /// Subscriptions whose next invoice cannot be computed right now.
    pub upcoming_unavailable: Vec<SubscriptionId>,
    /// How long the credit lasts for each live subscription, per wallet.
    pub outlooks: Vec<crate::domain::credit_outlook::CreditOutlook>,
}

#[derive(Debug, Clone)]
pub struct PortalUpcomingInvoice {
    pub details: crate::domain::SubscriptionDetails,
    pub invoice_date: NaiveDate,
    pub content: crate::services::invoice_lines::invoice_lines::ComputedInvoiceContent,
    /// The credit it draws is shared with other pending documents, so it is an estimate ("≈").
    pub credit_estimate_shared: bool,
    pub billed_first: Vec<crate::domain::credit_outlook::BilledFirst>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalLimitOrigin {
    Plan,
    Account,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortalLimitGroupUsage {
    pub group: BTreeMap<String, String>,
    pub used: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortalUsageLimit {
    pub feature_id: FeatureId,
    pub name: String,
    pub origin: PortalLimitOrigin,
    pub group_by: Option<String>,
    pub limit: Decimal,
    pub hard: bool,
    pub window_start: Option<DateTime<Utc>>,
    pub resets_at: Option<DateTime<Utc>>,
    /// Highest usage first.
    pub groups: Vec<PortalLimitGroupUsage>,
    /// Usage split by the metric's own dimensions, highest first.
    pub breakdown: Vec<PortalLimitGroupUsage>,
    pub reached: bool,
    /// When usage first reached the limit in this window, as its crossing was recorded.
    pub reached_at: Option<DateTime<Utc>>,
    /// A group is at or over the warning threshold, or at the limit when there is none.
    pub warning: bool,
    /// The limited metric's unit, when it names one.
    pub unit_singular: Option<String>,
    pub unit_plural: Option<String>,
}

/// What a subscription's recurring price is computed from, beyond its plan components.
#[derive(Debug, Clone, Default)]
pub struct PortalPlanInputs {
    /// The components in effect today.
    pub components: Vec<SubscriptionComponent>,
    /// Seats in use, by slot unit.
    pub slots: HashMap<String, u32>,
    pub minimum_minor: Option<i64>,
    /// The coupons still applying to its invoices, oldest first.
    pub coupons: Vec<AppliedCouponDetailed>,
    pub metrics: HashMap<BillableMetricId, BillableMetric>,
}

pub fn component_in_effect(component: &SubscriptionComponent, today: NaiveDate) -> bool {
    component.effective_from <= today && component.effective_to.is_none_or(|to| today < to)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalDiscount {
    pub code: String,
    pub percentage: Option<Decimal>,
    pub amount_minor: Option<i64>,
}

/// `list_minor` after the coupons, applied in order, with what each took off. A fixed coupon in
/// another currency does not apply; none takes the amount below zero.
pub fn apply_discounts(
    list_minor: i64,
    currency: &str,
    precision: u8,
    coupons: &[AppliedCouponDetailed],
) -> (i64, Vec<PortalDiscount>) {
    let mut net = list_minor;
    let mut discounts = Vec::new();
    for coupon in coupons {
        match &coupon.coupon.discount {
            CouponDiscount::Percentage(pct) => {
                let off = (Decimal::from(net) * pct / Decimal::ONE_HUNDRED)
                    .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
                    .to_subunit_opt(0)
                    .unwrap_or(0)
                    .clamp(0, net.max(0));
                net -= off;
                discounts.push(PortalDiscount {
                    code: coupon.coupon.code.clone(),
                    percentage: Some(*pct),
                    amount_minor: None,
                });
            }
            CouponDiscount::Fixed {
                currency: coupon_currency,
                amount,
            } if coupon_currency.eq_ignore_ascii_case(currency) => {
                let left = if coupon.coupon.applies_once() {
                    amount - coupon.applied_coupon.applied_amount.unwrap_or_default()
                } else {
                    *amount
                };
                let off = left
                    .to_subunit_opt(precision)
                    .unwrap_or(0)
                    .clamp(0, net.max(0));
                net -= off;
                discounts.push(PortalDiscount {
                    code: coupon.coupon.code.clone(),
                    percentage: None,
                    amount_minor: Some(off),
                });
            }
            CouponDiscount::Fixed { .. } => {}
            // Promotional credit funds a grant when redeemed; it never discounts an invoice.
            CouponDiscount::Credit { .. } => {}
        }
    }
    (net, discounts)
}

/// When a recurring invoice dated `invoice_date` is finalized, and charged when charged
/// automatically: after the entity's grace period, as billing schedules it.
pub fn recurring_finalization_at(
    invoice_date: NaiveDate,
    grace_period_hours: i32,
    consolidate: bool,
) -> NaiveDateTime {
    let hours =
        crate::services::invoices::bill::recurring_grace_hours(grace_period_hours, consolidate);
    invoice_date.and_time(NaiveTime::MIN) + chrono::Duration::hours(i64::from(hours))
}

/// The day a recurring invoice billed on `invoice_date` is issued (and charged when charged
/// automatically), and the day it is due: finalization re-dates it to its issue day and rebases
/// the due date on it (`Invoice::dates_if_issued_on`).
pub fn recurring_issue_and_due(
    invoice_date: NaiveDate,
    net_terms: u32,
    grace_period_hours: i32,
    consolidate: bool,
) -> (NaiveDate, NaiveDate) {
    let issued_on = recurring_finalization_at(invoice_date, grace_period_hours, consolidate).date();
    (
        issued_on,
        issued_on + chrono::Duration::days(i64::from(net_terms)),
    )
}

pub type PortalPlanInputsBySubscription = HashMap<SubscriptionId, PortalPlanInputs>;

#[cfg(test)]
mod recurring_issue_tests {
    use super::recurring_issue_and_due;
    use chrono::NaiveDate;

    #[test]
    fn a_full_day_of_grace_issues_and_dues_a_day_later() {
        let nov = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let dec = |d| NaiveDate::from_ymd_opt(2026, 12, d).unwrap();
        assert_eq!(
            recurring_issue_and_due(nov(1), 30, 24, false),
            (nov(2), dec(2))
        );
        assert_eq!(
            recurring_issue_and_due(nov(1), 30, 12, false),
            (nov(1), dec(1))
        );
        assert_eq!(
            recurring_issue_and_due(nov(1), 0, 0, true),
            (nov(1), nov(1))
        );
    }
}
