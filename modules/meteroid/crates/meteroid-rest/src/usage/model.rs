use chrono::NaiveDate;
use common_domain::ids::{BillableMetricId, SubscriptionId, TenantId, string_serde};
use meteroid_rest_common::datetime::ApiDateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::usage_costs::model::UsageCost;
use meteroid_rest_common::empty_string_as_none;
use std::sync::Arc;

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct CustomerUsageQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub metric_id: Option<BillableMetricId>,
}

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct SubscriptionUsageQuery {
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub metric_id: Option<BillableMetricId>,
}

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct UsageSummaryQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub metric_id: Option<BillableMetricId>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize)]
pub struct UsageResponse {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub usage: Vec<MetricUsage>,
    /// Beta: when `totals` and the metrics' `costs` were computed. These are present only when a
    /// subscription's current billing period is read (no `start_date` nor `end_date`) and its
    /// usage could be priced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computed_at: Option<ApiDateTime>,
    /// Beta: the sum of the metrics' `costs`, per currency or credit unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals: Option<Vec<UsageCost>>,
    /// v2 only: the metrics' `projected_costs` added up.
    #[serde(skip)]
    pub projected_totals: Option<Vec<UsageCost>>,
}

impl UsageResponse {
    pub fn without_costs(mut self) -> Self {
        self.computed_at = None;
        self.totals = None;
        self.projected_totals = None;
        for metric in &mut self.usage {
            metric.costs = None;
            metric.projected_value = None;
            metric.projected_costs = None;
        }
        self
    }

    pub fn only_metric(&self, metric_id: Option<BillableMetricId>) -> Self {
        let Some(metric_id) = metric_id else {
            return self.clone();
        };
        let usage: Vec<MetricUsage> = self
            .usage
            .iter()
            .filter(|metric| metric.metric_id == metric_id)
            .cloned()
            .collect();
        let totals = self.totals.as_ref().map(|_| {
            sum_costs(
                usage
                    .iter()
                    .flat_map(|metric| metric.costs.iter().flatten()),
            )
        });
        let projected_totals = projected_totals(&usage);
        Self {
            usage,
            totals,
            projected_totals,
            ..self.clone()
        }
    }
}

pub fn sum_costs<'a>(costs: impl IntoIterator<Item = &'a UsageCost>) -> Vec<UsageCost> {
    let mut totals: Vec<UsageCost> = Vec::new();
    for cost in costs {
        match totals
            .iter_mut()
            .find(|total| total.currency == cost.currency)
        {
            Some(total) => total.amount += cost.amount,
            None => totals.push(cost.clone()),
        }
    }
    totals
}

/// The metrics' projected costs added up; `None` unless every metric has one.
pub fn projected_totals(usage: &[MetricUsage]) -> Option<Vec<UsageCost>> {
    let mut projected = Vec::with_capacity(usage.len());
    for metric in usage {
        projected.push(metric.projected_costs.as_ref()?);
    }
    Some(sum_costs(projected.into_iter().flatten()))
}

/// A subscription's current-period usage and costs, shared by the reads of the next seconds.
pub type SubscriptionUsageCache =
    moka::future::Cache<(TenantId, SubscriptionId), Arc<UsageResponse>>;

pub const SUBSCRIPTION_USAGE_TTL_SECS: u64 = 30;

/// Readings are weighed by their usage groups, so a cache of high-cardinality metrics stays small.
pub fn new_subscription_usage_cache() -> SubscriptionUsageCache {
    moka::future::Cache::builder()
        .weigher(|_, reading: &Arc<UsageResponse>| {
            let groups: usize = reading
                .usage
                .iter()
                .map(|metric| 1 + metric.grouped_usage.len())
                .sum();
            u32::try_from(groups).unwrap_or(u32::MAX)
        })
        .max_capacity(1_000_000)
        .time_to_live(std::time::Duration::from_secs(SUBSCRIPTION_USAGE_TTL_SECS))
        .build()
}

#[derive(Clone, ToSchema, Serialize, Deserialize)]
pub struct MetricUsage {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub metric_name: String,
    pub metric_code: String,
    #[schema(value_type = String, format = "decimal")]
    pub total_value: rust_decimal::Decimal,
    pub grouped_usage: Vec<GroupedUsage>,
    /// Beta: what the metric's usage adds so far to the subscription's next invoice, per currency
    /// or credit unit; empty while it bills nothing, as during a free trial or once cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub costs: Option<Vec<UsageCost>>,
    /// v2 only: `total_value` at its pace until the period's end.
    #[serde(skip)]
    pub projected_value: Option<rust_decimal::Decimal>,
    /// v2 only: what `costs` would reach at the usage's pace until the period's end.
    #[serde(skip)]
    pub projected_costs: Option<Vec<UsageCost>>,
}

#[derive(Clone, ToSchema, Serialize, Deserialize)]
pub struct GroupedUsage {
    #[schema(value_type = String, format = "decimal")]
    pub value: rust_decimal::Decimal,
    pub dimensions: HashMap<String, String>,
}

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct UnattributedUsageQuery {
    /// First day of the range. Defaults to the first day of the current month (UTC).
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    /// Day after the range (exclusive). Defaults to the first day of the next month.
    #[serde(default)]
    pub end_date: Option<NaiveDate>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct UnattributedUsageResponse {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub usage: Vec<UnattributedMetricUsage>,
}

/// A metric's usage over `[from, until)` that no subscription bills: only subscriptions for
/// resources price the metric, and the usage is attributed to none of their resources.
#[derive(ToSchema, Serialize, Deserialize)]
pub struct UnattributedMetricUsage {
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub metric_name: String,
    pub metric_code: String,
    pub from: ApiDateTime,
    pub until: ApiDateTime,
    #[schema(value_type = String, format = "decimal")]
    pub total_value: rust_decimal::Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_costs::model::UsageCostUnit;
    use common_domain::ids::BaseId;
    use rust_decimal::Decimal;

    fn eur(amount: Decimal) -> UsageCost {
        UsageCost {
            amount,
            currency: "EUR".to_owned(),
            unit: UsageCostUnit::Currency,
        }
    }

    fn metric(projected: Option<Decimal>) -> MetricUsage {
        MetricUsage {
            metric_id: BillableMetricId::new(),
            metric_name: "Tokens".to_owned(),
            metric_code: "tokens".to_owned(),
            total_value: Decimal::from(10),
            grouped_usage: Vec::new(),
            costs: Some(vec![eur(Decimal::from(1))]),
            projected_value: projected.map(|_| Decimal::from(30)),
            projected_costs: projected.map(|amount| vec![eur(amount)]),
        }
    }

    fn reading(usage: Vec<MetricUsage>) -> UsageResponse {
        let projected_totals = projected_totals(&usage);
        UsageResponse {
            period_start: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            usage,
            computed_at: None,
            totals: None,
            projected_totals,
        }
    }

    #[test]
    fn a_closed_gate_leaves_no_projection() {
        let closed = reading(vec![metric(Some(Decimal::from(3)))]).without_costs();
        assert!(closed.projected_totals.is_none());
        assert!(closed.usage[0].projected_value.is_none());
        assert!(closed.usage[0].projected_costs.is_none());
    }

    #[test]
    fn one_metric_keeps_its_projection_when_another_has_none() {
        let projected = metric(Some(Decimal::from(3)));
        let id = projected.metric_id;
        let full = reading(vec![projected, metric(None)]);
        assert!(full.projected_totals.is_none());
        let one = full.only_metric(Some(id));
        assert_eq!(one.projected_totals, Some(vec![eur(Decimal::from(3))]));
    }
}
