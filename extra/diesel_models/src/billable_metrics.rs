
use chrono::NaiveDateTime;
use uuid::Uuid;


use diesel::{Identifiable, Queryable};
use diesel::sql_types::Nullable;
use crate::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};


#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::billable_metric)]
pub struct BillableMetric {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<Nullable<UnitConversionRoundingEnum>>,
    pub segmentation_matrix: Option<serde_json::Value>,
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
}
