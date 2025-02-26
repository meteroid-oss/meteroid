use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, ProductFamilyId, TenantId};
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::product_families::ProductFamilyRowNew;
use o2o::o2o;

#[derive(Clone, Debug, o2o)]
#[from_owned(ProductFamilyRow)]
#[owned_into(ProductFamilyRow)]
pub struct ProductFamily {
    pub id: ProductFamilyId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(ProductFamilyRowNew)]
#[ghosts(id: {ProductFamilyId::new() })]
pub struct ProductFamilyNew {
    pub name: String,
    pub tenant_id: TenantId,
}
