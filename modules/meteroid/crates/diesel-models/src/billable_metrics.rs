use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};

use common_domain::ids::{BillableMetricId, ProductFamilyId, ProductId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::billable_metric)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BillableMetricRow {
    pub id: BillableMetricId,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<serde_json::Value>,
    pub usage_group_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub product_id: Option<ProductId>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = crate::schema::billable_metric)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BillableMetricRowNew {
    pub id: BillableMetricId,
    pub name: String,
    pub description: Option<String>,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub unit_conversion_factor: Option<i32>,
    pub unit_conversion_rounding: Option<UnitConversionRoundingEnum>,
    pub segmentation_matrix: Option<serde_json::Value>,
    pub usage_group_key: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub product_id: Option<ProductId>,
}

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::billable_metric)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BillableMetricMetaRow {
    pub id: BillableMetricId,
    pub name: String,
    pub code: String,
    pub aggregation_type: BillingMetricAggregateEnum,
    pub aggregation_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}
