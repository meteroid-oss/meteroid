use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{PlanStatusEnum, PlanTypeEnum};
use crate::plan_versions::PlanVersionRow;
use common_domain::ids::{PlanId, ProductFamilyId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanRow {
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub active_version_id: Option<Uuid>,
    pub draft_version_id: Option<Uuid>,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanRowNew {
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

#[derive(Debug, Queryable)]
pub struct PlanRowOverview {
    pub id: PlanId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    pub product_family_name: String,
    pub product_family_id: ProductFamilyId,
    pub active_version: Option<PlanVersionRowInfo>,
    pub draft_version: Option<Uuid>,
    pub subscription_count: Option<i64>,
}

#[derive(Debug, Queryable)]
pub struct PlanVersionRowInfo {
    pub version: i32,
    pub trial_duration_days: Option<i32>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanWithVersionRow {
    #[diesel(embed)]
    pub plan: PlanRow,
    #[diesel(embed)]
    pub version: Option<PlanVersionRow>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct PlanRowPatch {
    pub id: PlanId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub active_version_id: Option<Option<Uuid>>,
    pub draft_version_id: Option<Option<Uuid>>,
}

pub struct PlanFilters {
    pub search: Option<String>,
    pub filter_status: Vec<PlanStatusEnum>,
    pub filter_type: Vec<PlanTypeEnum>,
}
