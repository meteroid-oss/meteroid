use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use chrono::NaiveDateTime;
use common_domain::identifiers::validator_code;
use common_domain::ids::{
    BillableMetricId, ProductFamilyId, ProductId, string_serde, string_serde_opt,
};
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
    Up,
    Down,
    Nearest,
    NearestHalf,
    NearestDecile,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_group_key: Option<String>,
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub product_family_id: ProductFamilyId,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub product_id: Option<ProductId>,
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::api_rest::model::serialize_datetime_opt"
    )]
    pub archived_at: Option<NaiveDateTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct UnitConversion {
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
    #[serde(serialize_with = "crate::api_rest::model::serialize_datetime")]
    pub created_at: NaiveDateTime,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::api_rest::model::serialize_datetime_opt"
    )]
    pub archived_at: Option<NaiveDateTime>,
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
    pub usage_group_key: Option<String>,
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
