use chrono::NaiveDateTime;
use o2o::o2o;
use uuid::Uuid;

use diesel_models::product_families::ProductFamily as DieselProductFamily;
use diesel_models::product_families::ProductFamilyNew as DieselProductFamilyNew;

#[derive(Clone, Debug, o2o)]
#[from_owned(DieselProductFamily)]
#[owned_into(DieselProductFamily)]
pub struct ProductFamily {
    pub id: Uuid,
    pub name: String,
    pub external_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
}

#[derive(Clone, Debug, o2o)]
#[owned_into(DieselProductFamilyNew)]
#[ghosts(id: {uuid::Uuid::now_v7()})]
pub struct ProductFamilyNew {
    pub name: String,
    pub external_id: String,
    pub tenant_id: Uuid,
}
