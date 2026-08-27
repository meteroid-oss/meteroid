use crate::StoreResult;
use crate::domain::tax_categories::TaxCategory;
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, TaxCategoryId, TenantId};
use diesel_models::tax_categories::{TaxCategoryRow, TaxCategoryRowNew};
use error_stack::Report;

#[async_trait::async_trait]
pub trait TaxCategoryInterface {
    /// Built-in categories plus the tenant's own, ordered by name.
    async fn list_tax_categories(&self, tenant_id: TenantId) -> StoreResult<Vec<TaxCategory>>;

    /// Creates a tenant-owned custom category. `key` is derived from the name.
    async fn create_tax_category(
        &self,
        tenant_id: TenantId,
        name: String,
        parent_id: Option<TaxCategoryId>,
    ) -> StoreResult<TaxCategory>;

    /// Renames/reparents a tenant-owned custom category. The stable `key` is never
    /// changed (external providers map on it). Built-in categories cannot be edited.
    async fn update_tax_category(
        &self,
        tenant_id: TenantId,
        id: TaxCategoryId,
        name: String,
        parent_id: Option<TaxCategoryId>,
    ) -> StoreResult<TaxCategory>;

    /// Deletes a tenant-owned custom category (built-ins cannot be deleted).
    /// References from products/custom taxes/entities are set to NULL by FK.
    async fn delete_tax_category(&self, tenant_id: TenantId, id: TaxCategoryId) -> StoreResult<()>;
}

/// Derives a stable, url-safe key from a category name (lowercased, non-alnum
/// runs collapsed to `_`). Uniqueness is per tenant, enforced by the DB.
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.extend(ch.to_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            slug.push('_');
            last_underscore = true;
        }
    }
    slug.trim_matches('_').to_string()
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

    async fn create_tax_category(
        &self,
        tenant_id: TenantId,
        name: String,
        parent_id: Option<TaxCategoryId>,
    ) -> StoreResult<TaxCategory> {
        let mut conn = self.get_conn().await?;

        let key = slugify(&name);
        if key.is_empty() {
            return Err(Report::new(StoreError::InvalidArgument(
                "tax category name must contain at least one alphanumeric character".to_string(),
            )));
        }

        if let Some(parent_id) = parent_id {
            validate_parent(&mut conn, parent_id, tenant_id).await?;
        }

        let row = TaxCategoryRowNew {
            id: TaxCategoryId::new(),
            tenant_id: Some(tenant_id),
            parent_id,
            key,
            name,
            is_builtin: false,
        };

        let inserted = row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(inserted.into())
    }

    async fn update_tax_category(
        &self,
        tenant_id: TenantId,
        id: TaxCategoryId,
        name: String,
        parent_id: Option<TaxCategoryId>,
    ) -> StoreResult<TaxCategory> {
        let mut conn = self.get_conn().await?;

        if name.trim().is_empty() {
            return Err(Report::new(StoreError::InvalidArgument(
                "tax category name is required".to_string(),
            )));
        }
        if parent_id == Some(id) {
            return Err(Report::new(StoreError::InvalidArgument(
                "a tax category cannot be its own parent".to_string(),
            )));
        }
        if let Some(parent_id) = parent_id {
            validate_parent(&mut conn, parent_id, tenant_id).await?;
        }

        let updated = TaxCategoryRow::update_details(&mut conn, id, tenant_id, name, parent_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        updated.map(Into::into).ok_or_else(|| {
            Report::new(StoreError::ValueNotFound(
                "tax category not found or is built-in".to_string(),
            ))
        })
    }

    async fn delete_tax_category(&self, tenant_id: TenantId, id: TaxCategoryId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let deleted = TaxCategoryRow::delete(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if deleted == 0 {
            return Err(Report::new(StoreError::ValueNotFound(
                "tax category not found or is built-in".to_string(),
            )));
        }
        Ok(())
    }
}

async fn validate_parent(
    conn: &mut crate::store::PgConn,
    parent_id: TaxCategoryId,
    tenant_id: TenantId,
) -> StoreResult<()> {
    let available = TaxCategoryRow::is_available_for_tenant(conn, parent_id, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if !available {
        return Err(Report::new(StoreError::InvalidArgument(
            "parent tax category is not available to this tenant".to_string(),
        )));
    }
    Ok(())
}
