//! The tenant's managed quantities. Created with the seat products and ledgers that use them;
//! their code is immutable, their name and policies can change.
use crate::StoreResult;
use crate::domain::managed_quantities::{ManagedQuantity, ManagedQuantityPatch};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{FeatureId, ManagedQuantityId, TenantId};
use diesel_models::entitlements::FeatureRow;
use diesel_models::enums::FeatureTypeEnum;
use diesel_models::managed_quantities::{ManagedQuantityRow, ManagedQuantityRowPatch};
use diesel_models::query::managed_quantities::{list_feature_limits, set_feature_limit_quantity};
use error_stack::Report;

#[async_trait::async_trait]
pub trait ManagedQuantitiesInterface {
    /// Ordered by code.
    async fn list_managed_quantities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ManagedQuantity>>;

    async fn update_managed_quantity(
        &self,
        patch: ManagedQuantityPatch,
    ) -> StoreResult<ManagedQuantity>;

    /// Counts the metered `feature`'s limit per unit of `quantity` a subscription holds, or per
    /// subscription again when none.
    async fn set_feature_limit_quantity(
        &self,
        tenant_id: TenantId,
        feature: FeatureId,
        quantity: Option<ManagedQuantityId>,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl ManagedQuantitiesInterface for Store {
    async fn list_managed_quantities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<ManagedQuantity>> {
        let mut conn = self.get_conn().await?;
        let mut quantities: Vec<ManagedQuantity> = ManagedQuantityRow::list(&mut conn, tenant_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        for (quantity, feature) in list_feature_limits(&mut conn, tenant_id).await? {
            if let Some(found) = quantities.iter_mut().find(|q| q.id == quantity) {
                found.per_unit_features.push(feature);
            }
        }
        Ok(quantities)
    }

    async fn update_managed_quantity(
        &self,
        patch: ManagedQuantityPatch,
    ) -> StoreResult<ManagedQuantity> {
        let mut conn = self.get_conn().await?;
        let row_patch: ManagedQuantityRowPatch = (&patch).into();
        Ok(
            ManagedQuantityRow::update(&mut conn, patch.tenant_id, patch.id, &row_patch)
                .await?
                .into(),
        )
    }

    async fn set_feature_limit_quantity(
        &self,
        tenant_id: TenantId,
        feature: FeatureId,
        quantity: Option<ManagedQuantityId>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let found = FeatureRow::find_by_id(&mut conn, feature, tenant_id).await?;
        if found.feature.feature_type != FeatureTypeEnum::Metered {
            return Err(Report::new(StoreError::InvalidArgument(
                "only a metered feature's limit counts per unit of a quantity".to_owned(),
            )));
        }
        if let Some(quantity) = quantity {
            let known = ManagedQuantityRow::list(&mut conn, tenant_id)
                .await?
                .iter()
                .any(|row| row.id == quantity && row.archived_at.is_none());
            if !known {
                return Err(Report::new(StoreError::ValueNotFound(format!(
                    "managed quantity {quantity}"
                ))));
            }
        }
        Ok(set_feature_limit_quantity(&mut conn, feature, quantity).await?)
    }
}
