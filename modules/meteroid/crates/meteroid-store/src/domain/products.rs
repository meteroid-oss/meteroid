use chrono::NaiveDateTime;
use common_domain::ids::{ProductFamilyId, ProductId, TenantId};
use diesel_models::products::ProductRow;
use o2o::o2o;
use uuid::Uuid;

#[derive(Clone, Debug, o2o)]
#[from_owned(ProductRow)]
#[owned_into(ProductRow)]
pub struct Product {
    pub id: ProductId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Uuid,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub product_family_id: ProductFamilyId,
}

#[derive(Clone, Debug)]
pub struct ProductNew {
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub family_id: ProductFamilyId,
}
