use common_domain::identifiers::validator_code;
use common_domain::ids::{
    BillableMetricId, ProductFamilyId, ProductId, string_serde, string_serde_opt,
};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

fn validate_metric_code(code: &str) -> Result<(), validator::ValidationError> {
    validator_code(code)
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::BillingMetricAggregateEnum)]
pub enum BillingMetricAggregateEnum {
    Count,
    Latest,
    Max,
    Min,
    Mean,
    Sum,
    CountDistinct,
}

#[derive(o2o, Serialize, Deserialize, Debug, Clone, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::enums::UnitConversionRoundingEnum)]
pub enum UnitConversionRoundingEnum {
    /// Up to the next whole unit.
    Up,
    /// Down to the whole unit.
    Down,
    /// To the nearest whole unit, halves away from zero.
    Nearest,
    /// To the nearest 0.5 unit, halves away from zero.
    NearestHalf,
    /// To the nearest 0.1 unit, halves away from zero.
    NearestDecile,
    /// Not rounded.
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, o2o, utoipa::ToSchema)]
#[map_owned(meteroid_store::domain::billable_metrics::Dimension)]
pub struct MetricDimension {
    pub key: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DoubleSegmentationMatrix {
    pub dimension1: MetricDimension,
    pub dimension2: MetricDimension,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LinkedSegmentationMatrix {
    pub dimension1_key: String,
    pub dimension2_key: String,
    pub values: HashMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetricSegmentationMatrix {
    Single(MetricDimension),
    Double(DoubleSegmentationMatrix),
    Linked(LinkedSegmentationMatrix),
}

impl From<meteroid_store::domain::billable_metrics::SegmentationMatrix>
    for MetricSegmentationMatrix
{
    fn from(val: meteroid_store::domain::billable_metrics::SegmentationMatrix) -> Self {
        match val {
            meteroid_store::domain::billable_metrics::SegmentationMatrix::Single(d) => {
                Self::Single(d.into())
            }
            meteroid_store::domain::billable_metrics::SegmentationMatrix::Double {
                dimension1,
                dimension2,
            } => Self::Double(DoubleSegmentationMatrix {
                dimension1: dimension1.into(),
                dimension2: dimension2.into(),
            }),
            meteroid_store::domain::billable_metrics::SegmentationMatrix::Linked {
                dimension1_key,
                dimension2_key,
                values,
            } => Self::Linked(LinkedSegmentationMatrix {
                dimension1_key,
                dimension2_key,
                values,
            }),
        }
    }
}

impl From<MetricSegmentationMatrix>
    for meteroid_store::domain::billable_metrics::SegmentationMatrix
{
    fn from(val: MetricSegmentationMatrix) -> Self {
        match val {
            MetricSegmentationMatrix::Single(d) => Self::Single(d.into()),
            MetricSegmentationMatrix::Double(d) => Self::Double {
                dimension1: d.dimension1.into(),
                dimension2: d.dimension2.into(),
            },
            MetricSegmentationMatrix::Linked(l) => Self::Linked {
                dimension1_key: l.dimension1_key,
                dimension2_key: l.dimension2_key,
                values: l.values,
            },
        }
    }
}

/// Operator of a pre-aggregation [`MetricFilter`]. `EQUAL`/`NOT_EQUAL` are the single-value
/// forms of `IN`/`NOT_IN`. Negation (`NOT_EQUAL`/`NOT_IN`) is presence-required: an event
/// missing the property is excluded.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::billable_metrics::MetricFilterOp)]
pub enum MetricFilterOperator {
    Equal,
    NotEqual,
    In,
    NotIn,
}

/// A pre-aggregation filter: only events whose `property` matches feed the metric's
/// aggregation. Distinct from a segmentation dimension (which splits pricing). Multiple
/// filters are ANDed.
#[derive(o2o, Serialize, Deserialize, Debug, Clone, utoipa::ToSchema)]
#[map_owned(meteroid_store::domain::billable_metrics::MetricFilter)]
pub struct MetricFilter {
    pub property: String,
    #[map(~.into())]
    pub op: MetricFilterOperator,
    pub values: Vec<String>,
}

impl MetricFilter {
    /// Reject a filter with no property or no values — a no-op filter would otherwise
    /// silently meter unfiltered. Mirrors the gRPC validation.
    pub fn validate_all(filters: &[MetricFilter]) -> Result<(), String> {
        for f in filters {
            if f.property.trim().is_empty() {
                return Err("metric filter property must not be empty".to_string());
            }
            if f.values.is_empty() {
                return Err(format!(
                    "metric filter on '{}' must have at least one value",
                    f.property
                ));
            }
        }
        Ok(())
    }
}

// ── Response types ─────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Metric {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: BillableMetricId,
    pub name: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub aggregation_type: BillingMetricAggregateEnum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_conversion: Option<UnitConversion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segmentation_matrix: Option<MetricSegmentationMatrix>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub filters: Vec<MetricFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_group_key: Option<String>,
    /// What one unit of the metric is called, e.g. `token`. Set together with `unit_plural`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_singular: Option<String>,
    /// What several units of the metric are called, e.g. `tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_plural: Option<String>,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub product_family_id: ProductFamilyId,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub product_id: Option<ProductId>,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
}

/// Converts the aggregated usage into the unit prices are set in, e.g. tokens priced per 1M.
/// The usage is divided by `factor`, then rounded. Rounding applies to each priced group's
/// total for the billing period: each group of a segmented or grouped metric, and each matrix
/// cell, is rounded on its own, never each event.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct UnitConversion {
    /// Raw units per priced unit, e.g. `1000000` to price per 1M.
    pub factor: i32,
    pub rounding: UnitConversionRoundingEnum,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MetricSummary {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: BillableMetricId,
    pub name: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub aggregation_type: BillingMetricAggregateEnum,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation_key: Option<String>,
    /// What one unit of the metric is called, e.g. `token`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_singular: Option<String>,
    /// What several units of the metric are called, e.g. `tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_plural: Option<String>,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
}

// ── Request types ──────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct CreateMetricRequest {
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1), custom(function = "validate_metric_code"))]
    pub code: String,
    pub description: Option<String>,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion: Option<UnitConversion>,
    pub segmentation_matrix: Option<MetricSegmentationMatrix>,
    /// Pre-aggregation property filters. Optional and backward-compatible; omit for none.
    #[serde(default)]
    pub filters: Option<Vec<MetricFilter>>,
    pub usage_group_key: Option<String>,
    /// What one unit of the metric is called, e.g. `token`, used to label quantities and
    /// prices ("$12.00 per 1M tokens"). Set both `unit_singular` and `unit_plural`, or neither.
    /// Trimmed; at most 32 characters.
    #[schema(example = "token")]
    pub unit_singular: Option<String>,
    /// What several units of the metric are called, e.g. `tokens`. Set together with
    /// `unit_singular`. Trimmed; at most 32 characters.
    #[schema(example = "tokens")]
    pub unit_plural: Option<String>,
    #[serde(with = "string_serde")]
    pub product_family_id: ProductFamilyId,
    #[serde(default, with = "string_serde_opt")]
    pub product_id: Option<ProductId>,
}

#[derive(Clone, Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateMetricRequest {
    #[validate(length(min = 1))]
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub unit_conversion: Option<Option<UnitConversion>>,
    pub segmentation_matrix: Option<Option<MetricSegmentationMatrix>>,
    /// Absent = leave filters untouched; present (even empty) = replace them.
    #[serde(default)]
    pub filters: Option<Vec<MetricFilter>>,
    /// What one unit of the metric is called, e.g. `token`. Send `unit_singular` and
    /// `unit_plural` together; omit both to leave the label unchanged, or send both as empty
    /// strings to remove it. Trimmed; at most 32 characters.
    #[schema(example = "token")]
    pub unit_singular: Option<String>,
    /// What several units of the metric are called, e.g. `tokens`. See `unit_singular`.
    #[schema(example = "tokens")]
    pub unit_plural: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MetricListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(default, with = "string_serde_opt")]
    pub product_family_id: Option<ProductFamilyId>,
    /// Search by metric name or code
    pub search: Option<String>,
    /// Sort order. Format: `column.direction`. Allowed columns: `name`, `code`, `created_at`. Direction: `asc` or `desc`. Default: `name.asc`.
    pub order_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MetricListResponse {
    pub data: Vec<MetricSummary>,
    pub pagination_meta: PaginationResponse,
}
