use crate::StoreResult;
use crate::domain::tax_categories::TaxCategory;
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::TenantId;
use diesel_models::tax_categories::TaxCategoryRow;
use error_stack::Report;

#[async_trait::async_trait]
pub trait TaxCategoryInterface {
    /// Built-in categories plus the tenant's own, ordered by name.
    async fn list_tax_categories(&self, tenant_id: TenantId) -> StoreResult<Vec<TaxCategory>>;
}

#[async_trait::async_trait]
impl TaxCategoryInterface for Store {
    async fn list_tax_categories(&self, tenant_id: TenantId) -> StoreResult<Vec<TaxCategory>> {
        let mut conn = self.get_conn().await?;
        let rows = TaxCategoryRow::list_available(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
