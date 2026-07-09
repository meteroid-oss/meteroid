use chrono::NaiveDateTime;
use common_domain::ids::{TaxCategoryId, TenantId};
use diesel::{Identifiable, Insertable, Queryable, Selectable};

#[derive(Clone, Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::tax_category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TaxCategoryRow {
    pub id: TaxCategoryId,
    pub tenant_id: Option<TenantId>,
    pub parent_id: Option<TaxCategoryId>,
    pub key: String,
    pub name: String,
    pub is_builtin: bool,
    pub created_at: NaiveDateTime,
}

#[derive(Clone, Debug, Insertable)]
#[diesel(table_name = crate::schema::tax_category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TaxCategoryRowNew {
    pub id: TaxCategoryId,
    pub tenant_id: Option<TenantId>,
    pub parent_id: Option<TaxCategoryId>,
    pub key: String,
    pub name: String,
    pub is_builtin: bool,
}
