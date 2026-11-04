use crate::domain::BillableMetric;
use crate::services::clients::usage::{MetricEvent, MetricEventCursor};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::ids::{BillableMetricId, CustomerId, ProductId, SubscriptionId};
use rust_decimal::Decimal;

/// A billable metric named the way API callers do: by id or by code.
#[derive(Debug, Clone)]
pub enum MetricRef {
    Id(BillableMetricId),
    Code(String),
}

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
}

#[derive(Debug, Clone)]
pub struct RunningTotal {
    pub computed_at: NaiveDateTime,
    pub metrics: Vec<MetricRunningCost>,
}
