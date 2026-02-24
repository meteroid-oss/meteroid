use chrono::NaiveDate;
use common_domain::ids::{BillableMetricId, string_serde};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

use crate::api_rest::empty_string_as_none;

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct CustomerUsageQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    #[param(value_type = Option<String>)]
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
    #[param(value_type = Option<String>)]
    pub metric_id: Option<BillableMetricId>,
}

#[derive(ToSchema, IntoParams, Serialize, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct UsageSummaryQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    #[param(value_type = Option<String>)]
    pub metric_id: Option<BillableMetricId>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct UsageResponse {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub usage: Vec<MetricUsage>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct MetricUsage {
    #[schema(value_type = String)]
    #[serde(with = "string_serde")]
    pub metric_id: BillableMetricId,
    pub metric_name: String,
    pub metric_code: String,
    #[schema(value_type = String, format = "decimal")]
    pub total_value: rust_decimal::Decimal,
    pub grouped_usage: Vec<GroupedUsage>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct GroupedUsage {
    #[schema(value_type = String, format = "decimal")]
    pub value: rust_decimal::Decimal,
    pub dimensions: HashMap<String, String>,
}
