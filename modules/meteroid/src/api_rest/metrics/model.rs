use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use chrono::NaiveDateTime;
use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

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

#[derive(Clone, Debug, Serialize, Deserialize, o2o, utoipa::ToSchema)]
#[serde(tag = "discriminator", rename_all = "SCREAMING_SNAKE_CASE")]
#[map_owned(meteroid_store::domain::billable_metrics::SegmentationMatrix)]
pub enum MetricSegmentationMatrix {
    Single(#[map(~.into())] MetricDimension),
    Double {
        #[map(~.into())]
        dimension1: MetricDimension,
        #[map(~.into())]
        dimension2: MetricDimension,
    },
    Linked {
        dimension1_key: String,
        dimension2_key: String,
        values: HashMap<String, Vec<String>>,
    },
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
    #[validate(length(min = 1))]
    pub code: String,
    pub description: Option<String>,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion: Option<UnitConversion>,
    pub segmentation_matrix: Option<MetricSegmentationMatrix>,
    pub usage_group_key: Option<String>,
    pub product_family_id: ProductFamilyId,
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
    pub product_family_id: Option<ProductFamilyId>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MetricListResponse {
    pub data: Vec<MetricSummary>,
    pub pagination_meta: PaginationResponse,
}
