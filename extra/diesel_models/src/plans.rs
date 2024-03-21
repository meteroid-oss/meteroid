use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::{PlanStatusEnum, PlanTypeEnum};
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::plan)]
pub struct Plan {
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
pub struct PlanNew {
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
