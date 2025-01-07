use chrono::NaiveDateTime;
use diesel_models::products::ProductRow;
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(ProductRow)]
#[owned_into(ProductRow)]
pub struct Product {
    pub id: Uuid,
    pub local_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub product_family_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct ProductNew {
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: Uuid,
    pub family_local_id: String,
}
