use common_domain::ids::{TaxCategoryId, TenantId};
use diesel_models::tax_categories::TaxCategoryRow;
use o2o::o2o;

/// A provider-agnostic classification of what is sold. Built-in categories are
/// global (`tenant_id` is None); tenant-custom categories carry a tenant id.
#[derive(Debug, Clone, o2o)]
#[from_owned(TaxCategoryRow)]
pub struct TaxCategory {
    pub id: TaxCategoryId,
    pub tenant_id: Option<TenantId>,
    pub parent_id: Option<TaxCategoryId>,
    pub key: String,
    pub name: String,
    pub is_builtin: bool,
}
