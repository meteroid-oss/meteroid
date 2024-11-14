use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

use diesel_models::product_families::ProductFamilyRow;
use diesel_models::product_families::ProductFamilyRowNew;

#[derive(Clone, Debug, o2o)]
#[from_owned(ProductFamilyRow)]
#[owned_into(ProductFamilyRow)]
pub struct ProductFamily {
    pub id: Uuid,
    pub name: String,
    pub local_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(ProductFamilyRowNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct ProductFamilyNew {
    pub name: String,
    pub local_id: String,
    pub tenant_id: Uuid,
}
