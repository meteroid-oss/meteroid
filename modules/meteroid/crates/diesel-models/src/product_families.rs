use chrono::NaiveDateTime;

use common_domain::ids::{ProductFamilyId, TenantId};
use diesel::{Identifiable, Insertable, Queryable};

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::product_family)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductFamilyRow {
    pub id: ProductFamilyId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::product_family)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProductFamilyRowNew {
    pub id: ProductFamilyId,
    pub name: String,
    pub tenant_id: TenantId,
}
