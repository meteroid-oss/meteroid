use crate::domain::BillableMetric;
use crate::services::clients::usage::{MetricEvent, MetricEventCursor};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BillableMetricId, CustomerId, ProductId, SubscriptionId};
use rust_decimal::Decimal;
use std::collections::HashMap;

/// A billable metric named the way API callers do: by id or by code.
#[derive(Debug, Clone)]
pub enum MetricRef {
    Id(BillableMetricId),
    Code(String),
}

/// Attached to a usage cost read refused because the owner has no billing period pricing the
/// metric in the range: no fee bills that usage there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoBillingPeriod;

/// Whose usage a read covers: all a customer is billed for, or one subscription's share of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageOwner {
    Customer(CustomerId),
    Subscription(SubscriptionId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostGranularity {
    Hour,
    Day,
    Month,
}

/// What a cost is counted in: money, or the credit unit a credit-priced fee bills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostUnit {
    Currency,
    CreditUnit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageCost {
    pub amount: Decimal,
    /// A currency or a credit unit code, as `unit` says.
    pub currency: String,
    pub unit: CostUnit,
}

#[derive(Debug, Clone)]
pub struct MetricEventCost {
    pub event: MetricEvent,
    /// Billed units the event adds; `None` when the event does not add up to the metric.
    pub quantity: Option<Decimal>,
    pub cost: Option<UsageCost>,
}

#[derive(Debug, Clone)]
pub struct MetricEventsPage {
    pub events: Vec<MetricEventCost>,
    pub next: Option<MetricEventCursor>,
}

#[derive(Debug, Clone)]
pub struct UsageCostRow {
    pub bucket_start: NaiveDateTime,
    pub group: Option<String>,
    pub quantity: Decimal,
    pub cost: Option<UsageCost>,
    pub exact: bool,
}

/// Which usage windows a wallet's daily usage covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletUsageSpan {
    /// What each subscription's next invoice bills.
    CurrentPeriod,
    /// What the invoice opening each subscription's current period billed: its previous period's
    /// usage, fees a plan change closed mid-period included.
    PreviousPeriod,
    /// Every window overlapping the days `[from, to)`.
    Days { from: NaiveDate, to: NaiveDate },
}

/// What one day's usage of a metric costs the wallet, priced as its invoice prices it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletDailyUsageCost {
    pub date: NaiveDate,
    pub subscription_id: SubscriptionId,
    pub metric_id: BillableMetricId,
    pub metric_name: String,
    pub product_id: Option<ProductId>,
    /// Billed units the day added to the period's.
    pub quantity: Decimal,
    /// In the wallet's minor units, before tax.
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalletDailyUsage {
    /// The days the windows cover, `to` exclusive; unset without any window.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub rows: Vec<WalletDailyUsageCost>,
}

/// What a metric's usage adds so far to the subscription's next invoice, by currency or credit
/// unit, before discounts and credits.
#[derive(Debug, Clone)]
pub struct MetricRunningCost {
    pub metric: BillableMetric,
    pub costs: Vec<UsageCost>,
    /// Where the usage would end each window at its pace so far ([`project`]); `None` while a
    /// window is too young to tell, or has not started.
    pub projected: Option<MetricProjection>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricProjection {
    /// The windows' projected usage added up; `None` for a metric that does not add up over
    /// time, which is projected as it stands.
    pub quantity: Option<Decimal>,
    /// The windows priced whole at their projected usage. A credit-priced window counts its
    /// credits: the money overage once the wallet runs out is not projected.
    pub costs: Vec<UsageCost>,
}

/// Below this much of a window, a linear projection says more about noise than about usage.
pub const MIN_PROJECTION_ELAPSED_SECONDS: i64 = 3600;
/// Nor below this share of it, lest an early burst project a hundredfold.
const MIN_PROJECTION_ELAPSED_PERCENT: i64 = 5;

/// How much longer `[start, end)` is than what of it has elapsed at `at`; `None` too early to tell.
pub fn projection_factor(
    start: NaiveDateTime,
    end: NaiveDateTime,
    at: NaiveDateTime,
) -> Option<Decimal> {
    if at >= end {
        return Some(Decimal::ONE);
    }
    let elapsed = (at - start).num_seconds();
    let total = (end - start).num_seconds();
    if elapsed < MIN_PROJECTION_ELAPSED_SECONDS
        || elapsed * 100 < total * MIN_PROJECTION_ELAPSED_PERCENT
    {
        return None;
    }
    Decimal::from(total).checked_div(Decimal::from(elapsed))
}

/// `metric`'s `value` at the same pace over a range `factor` times longer: a metric that adds up
/// over time (sum, count) grows linearly, any other (max, latest, mean, unique) as it stands.
pub fn project(metric: &BillableMetric, value: Decimal, factor: Decimal) -> Option<Decimal> {
    match adds_up(metric) {
        true => value.checked_mul(factor),
        false => Some(value),
    }
}

pub fn adds_up(metric: &BillableMetric) -> bool {
    use crate::domain::enums::BillingMetricAggregateEnum as Aggregate;
    matches!(metric.aggregation_type, Aggregate::Sum | Aggregate::Count)
}

/// An event priced without being ingested.
#[derive(Debug, Clone)]
pub struct EstimatedEvent {
    pub code: String,
    pub timestamp: NaiveDateTime,
    pub properties: HashMap<String, String>,
}

/// What one metric of the event's code would bill for it on the subscription's next invoice.
#[derive(Debug, Clone)]
pub struct MetricEstimate {
    pub metric: BillableMetric,
    /// The billed units it adds; `None` when the metric does not add events up (max, latest,
    /// mean, unique).
    pub quantity: Option<Decimal>,
    /// What it adds to the invoice, by currency or credit unit, before discounts and credits;
    /// `None` with `quantity`.
    pub fees: Option<Vec<UsageCost>>,
}

#[derive(Debug, Clone)]
pub struct EventEstimate {
    pub computed_at: NaiveDateTime,
    /// The metrics that would bill the event; empty when none does.
    pub metrics: Vec<MetricEstimate>,
}

#[derive(Debug, Clone)]
pub struct RunningTotal {
    pub computed_at: NaiveDateTime,
    pub metrics: Vec<MetricRunningCost>,
}
