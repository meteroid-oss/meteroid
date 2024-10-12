use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{PlanStatusEnum, PlanTypeEnum};
use crate::plan_versions::PlanVersionRow;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanRowNew {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,

    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
}

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanRowForList {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
    pub external_id: String,
    pub plan_type: PlanTypeEnum,
    pub status: PlanStatusEnum,
    #[diesel(select_expression = crate::schema::product_family::name)]
    #[diesel(select_expression_type = crate::schema::product_family::name)]
    pub product_family_name: String,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PlanWithVersionRow {
    #[diesel(embed)]
    pub plan: PlanRow,
    #[diesel(embed)]
    pub version: PlanVersionRow,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::plan)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct PlanRowPatch {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
}

pub struct PlanFilters {
    pub search: Option<String>,
    pub filter_status: Option<PlanStatusEnum>,
    pub filter_type: Option<PlanTypeEnum>,
}
