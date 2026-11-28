//! The modules a tenant turned on, on top of what its organization's license grants: the
//! license decides whether a module can run, the tenant whether it does.
use crate::StoreResult;
use crate::store::Store;
use common_domain::ids::TenantId;
use diesel_models::tenant_modules::TenantModuleRow;
use error_stack::Report;

use crate::errors::StoreError;

#[async_trait::async_trait]
pub trait TenantModulesInterface {
    /// The ids of the modules the tenant turned on.
    async fn list_tenant_modules(&self, tenant_id: TenantId) -> StoreResult<Vec<String>>;

    async fn is_tenant_module_enabled(
        &self,
        tenant_id: TenantId,
        module: &str,
    ) -> StoreResult<bool>;

    /// Whether the module was on before.
    async fn set_tenant_module_enabled(
        &self,
        tenant_id: TenantId,
        module: &str,
        enabled: bool,
    ) -> StoreResult<bool>;
}

fn module_id(module: &str) -> StoreResult<&str> {
    let valid = !module.is_empty()
        && module.len() <= 64
        && module.starts_with(|c: char| c.is_ascii_lowercase())
        && module
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(module)
    } else {
        Err(Report::new(StoreError::InvalidArgument(format!(
            "not a module id: {module}"
        ))))
    }
}

#[async_trait::async_trait]
impl TenantModulesInterface for Store {
    async fn list_tenant_modules(&self, tenant_id: TenantId) -> StoreResult<Vec<String>> {
        let mut conn = self.get_conn().await?;
        Ok(TenantModuleRow::list(&mut conn, tenant_id)
            .await?
            .into_iter()
            .map(|row| row.module)
            .collect())
    }

    async fn is_tenant_module_enabled(
        &self,
        tenant_id: TenantId,
        module: &str,
    ) -> StoreResult<bool> {
        let mut conn = self.get_conn().await?;
        Ok(
            TenantModuleRow::find(&mut conn, tenant_id, module_id(module)?)
                .await?
                .is_some(),
        )
    }

    async fn set_tenant_module_enabled(
        &self,
        tenant_id: TenantId,
        module: &str,
        enabled: bool,
    ) -> StoreResult<bool> {
        let module = module_id(module)?;
        let mut conn = self.get_conn().await?;
        let before = TenantModuleRow::find(&mut conn, tenant_id, module)
            .await?
            .is_some();
        if enabled {
            TenantModuleRow::enable(&mut conn, tenant_id, module).await?;
        } else {
            TenantModuleRow::disable(&mut conn, tenant_id, module).await?;
        }
        Ok(before)
    }
}

#[cfg(test)]
mod tests {
    use super::module_id;

    #[test]
    fn module_ids_are_lowercase_slugs() {
        assert!(module_id("ai").is_ok());
        assert!(module_id("usage-controls").is_ok());
        assert!(module_id("").is_err());
        assert!(module_id("AI").is_err());
        assert!(module_id("1ai").is_err());
        assert!(module_id("ai module").is_err());
    }
}
