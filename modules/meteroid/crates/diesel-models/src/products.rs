use chrono::NaiveDateTime;

use crate::enums::FeeTypeEnum;
use common_domain::ids::{ProductFamilyId, ProductId, TaxCategoryId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Queryable, Debug, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::product)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductRow {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub fee_type: FeeTypeEnum,
    pub fee_structure: serde_json::Value,
    pub catalog: bool,
    pub tax_category_id: Option<TaxCategoryId>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::product)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductRowNew {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
    pub fee_type: FeeTypeEnum,
    pub fee_structure: serde_json::Value,
    pub catalog: bool,
    pub tax_category_id: Option<TaxCategoryId>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::product)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id, tenant_id))]
pub struct ProductRowPatch {
    pub id: ProductId,
    pub tenant_id: TenantId,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub fee_structure: Option<serde_json::Value>,
    pub tax_category_id: Option<Option<TaxCategoryId>>,
}
