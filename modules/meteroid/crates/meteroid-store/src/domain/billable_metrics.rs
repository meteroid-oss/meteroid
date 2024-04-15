use super::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};
use chrono::NaiveDateTime;

use diesel_models::billable_metrics::{
    BillableMetric as DieselBillableMetric, BillableMetricNew as DieselBillableMetricNew,
};
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(DieselBillableMetric)]
#[owned_into(DieselBillableMetric)]
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

#[derive(Clone, Debug, o2o)]
#[owned_into(DieselBillableMetricNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct BillableMetricNew {
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    #[into(~.into())]
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    #[into(~.map(|x| x.into()))]
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<serde_json::Value>,
    pub usage_group_key: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
}
