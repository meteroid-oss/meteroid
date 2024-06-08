use super::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};
use chrono::NaiveDateTime;

use diesel_models::billable_metrics::{BillableMetricMetaRow, BillableMetricRow};
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(BillableMetricRow)]
#[owned_into(BillableMetricRow)]
pub struct BillableMetric {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    #[map(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    #[map(~.map(|x| x.into()))]
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<serde_json::Value>,
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct BillableMetricNew {
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<serde_json::Value>, // todo refactor into structure
    pub usage_group_key: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub family_external_id: String,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(BillableMetricMetaRow)]
#[owned_into(BillableMetricMetaRow)]
pub struct BillableMetricMeta {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    #[map(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}
