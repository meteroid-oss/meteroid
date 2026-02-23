use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::FeeTypeEnum;
use common_domain::ids::{ProductFamilyId, ProductId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::product)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductRow {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub fee_type: FeeTypeEnum,
    pub fee_structure: serde_json::Value,
    pub catalog: bool,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::product)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductRowNew {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub fee_type: FeeTypeEnum,
    pub fee_structure: serde_json::Value,
    pub catalog: bool,
}
