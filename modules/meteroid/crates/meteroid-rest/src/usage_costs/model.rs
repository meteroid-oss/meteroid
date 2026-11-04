use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::usage_costs as domain;
use meteroid_store::services::clients::usage::MetricEventCursor;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUsageEventsRequest {
    /// Billable metric, by ID or code.
    #[validate(length(min = 1, max = 256))]
    pub metric: String,
    /// Start of the range, inclusive.
    pub from: ApiDateTime,
    /// End of the range, exclusive. At most 93 days after `from`.
    pub to: ApiDateTime,
    /// The `next_cursor` of the previous page.
    #[validate(length(max = 512))]
    pub cursor: Option<String>,
    /// Page size, 100 by default and 500 at most.
    #[validate(range(min = 1, max = 500))]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CostGranularity {
    Hour,
    Day,
    Month,
}

impl From<CostGranularity> for domain::CostGranularity {
    fn from(value: CostGranularity) -> Self {
        match value {
            CostGranularity::Hour => Self::Hour,
            CostGranularity::Day => Self::Day,
            CostGranularity::Month => Self::Month,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListUsageCostsRequest {
    /// Billable metric, by ID or code.
    #[validate(length(min = 1, max = 256))]
    pub metric: String,
    /// Event property to split the usage by.
    #[validate(length(min = 1, max = 128))]
    pub group_by: Option<String>,
    #[param(inline)]
    pub granularity: CostGranularity,
    /// Start of the range, inclusive.
    pub from: ApiDateTime,
    /// End of the range, exclusive. At most 93 days after `from`.
    pub to: ApiDateTime,
}

/// What a cost is counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UsageCostUnit {
    /// Money, in the subscription's currency.
    Currency,
    /// The credit unit a credit-priced fee bills; what credits leave uncovered is invoiced in
    /// money and is not part of the amount.
    CreditUnit,
}

/// Usage priced as its invoice line prices it, before discounts and credits: tax-inclusive when
/// the subscription's prices are.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct UsageCost {
    #[schema(value_type = String, format = "decimal")]
    pub amount: Decimal,
    /// A currency code, or a credit unit code when `unit` is `CREDIT_UNIT`.
    pub currency: String,
    pub unit: UsageCostUnit,
}

impl From<domain::UsageCost> for UsageCost {
    fn from(cost: domain::UsageCost) -> Self {
        Self {
            amount: cost.amount,
            currency: cost.currency,
            unit: match cost.unit {
                domain::CostUnit::Currency => UsageCostUnit::Currency,
                domain::CostUnit::CreditUnit => UsageCostUnit::CreditUnit,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UsageEvent {
    pub id: String,
    pub timestamp: ApiDateTime,
    pub properties: HashMap<String, String>,
    /// Billed units the event adds. Absent when the metric's aggregation cannot be split per
    /// event, or when the event does not count towards the metric.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub quantity: Option<Decimal>,
    /// The event's cost at the price in force, unrounded. Present for per-unit and matrix pricing
    /// only: absent for tiered, volume, package and capacity pricing, whose price depends on the
    /// whole period's usage, and for a metric whose unit conversion rounds.
    pub cost: Option<UsageCost>,
}

impl From<domain::MetricEventCost> for UsageEvent {
    fn from(item: domain::MetricEventCost) -> Self {
        Self {
            id: item.event.id,
            timestamp: item.event.timestamp.into(),
            properties: item.event.properties,
            quantity: item.quantity,
            cost: item.cost.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UsageEventListResponse {
    pub data: Vec<UsageEvent>,
    /// Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UsageCostRow {
    pub bucket_start: ApiDateTime,
    /// The value of `group_by` for the row. Absent when the request is not grouped, or the usage
    /// has no value for the property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: Decimal,
    /// Rounded to the currency's minor unit so that the rows of a billing period add up to the
    /// invoiced amount. Absent when the metric's aggregation cannot be priced per bucket.
    pub cost: Option<UsageCost>,
    /// False when the price depends on the whole period's usage (tiered, volume, package,
    /// capacity): the cost is then what the bucket adds to the period's price so far. A period's
    /// rows still add up to its invoiced amount.
    pub exact: bool,
}

impl From<domain::UsageCostRow> for UsageCostRow {
    fn from(row: domain::UsageCostRow) -> Self {
        Self {
            bucket_start: row.bucket_start.into(),
            group: row.group,
            quantity: row.quantity,
            cost: row.cost.map(Into::into),
            exact: row.exact,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UsageCostListResponse {
    pub data: Vec<UsageCostRow>,
}

#[derive(Serialize, Deserialize)]
struct CursorPayload {
    t: i64,
    i: String,
}

pub fn encode_cursor(cursor: &MetricEventCursor) -> String {
    let payload = CursorPayload {
        t: cursor.timestamp_nanos,
        i: cursor.id.clone(),
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap_or_default())
}

pub fn decode_cursor(raw: &str) -> Option<MetricEventCursor> {
    let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
    let payload: CursorPayload = serde_json::from_slice(&bytes).ok()?;
    Some(MetricEventCursor {
        timestamp_nanos: payload.t,
        id: payload.i,
    })
}
