use crate::StoreResult;
use crate::store::Store;
use error_stack::Report;

use crate::domain::price_components::{PriceComponent, PriceComponentNew};
use crate::errors::StoreError;
use common_domain::ids::{PlanVersionId, PriceComponentId, TenantId};
use diesel_models::price_components::PriceComponentRow;

#[async_trait::async_trait]
pub trait PriceComponentInterface {
    async fn list_price_components(
        &self,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PriceComponent>>;

    async fn get_price_component_by_id(
        &self,
        tenant_id: TenantId,
        id: PriceComponentId,
    ) -> StoreResult<PriceComponent>;

    async fn create_price_component(
        &self,
        price_component: PriceComponentNew,
    ) -> StoreResult<PriceComponent>;

    async fn create_price_component_batch(
        &self,
        price_component: Vec<PriceComponentNew>,
    ) -> StoreResult<Vec<PriceComponent>>;

    async fn update_price_component(
        &self,
        price_component: PriceComponent,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<Option<PriceComponent>>;

    async fn delete_price_component(
        &self,
        component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl PriceComponentInterface for Store {
    async fn list_price_components(
        &self,
        plan_version_id: PlanVersionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<PriceComponent>> {
        let mut conn = self.get_conn().await?;

        let components =
            PriceComponentRow::list_by_plan_version_id(&mut conn, tenant_id, plan_version_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        components
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()
    }

    async fn get_price_component_by_id(
        &self,
        tenant_id: TenantId,
        price_component_id: PriceComponentId,
    ) -> StoreResult<PriceComponent> {
        let mut conn = self.get_conn().await?;

        PriceComponentRow::get_by_id(&mut conn, tenant_id, price_component_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn create_price_component(
        &self,
        price_component: PriceComponentNew,
    ) -> StoreResult<PriceComponent> {
        let mut conn = self.get_conn().await?;
        let price_component = price_component.try_into()?;
        let inserted = PriceComponentRow::insert(&mut conn, price_component)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        inserted.try_into()
    }

    async fn create_price_component_batch(
        &self,
        price_components: Vec<PriceComponentNew>,
    ) -> StoreResult<Vec<PriceComponent>> {
        let mut conn = self.get_conn().await?;
        let price_components = price_components
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        let inserted = PriceComponentRow::insert_batch(&mut conn, price_components)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        inserted
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn update_price_component(
        &self,
        price_component: PriceComponent,
        tenant_id: TenantId,
        plan_version_id: PlanVersionId,
    ) -> StoreResult<Option<PriceComponent>> {
        let json_fee = serde_json::to_value(&price_component.fee).map_err(|e| {
            StoreError::SerdeError("Failed to serialize price component fee".to_string(), e)
        })?;

        let mut conn = self.get_conn().await?;
        let price_component: PriceComponentRow = PriceComponentRow {
            id: price_component.id,
            plan_version_id,
            name: price_component.name,
            product_id: price_component.product_id,
            fee: json_fee,
            billable_metric_id: price_component.fee.metric_id(),
        };
        let updated = price_component
            .update(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        match updated {
            None => Ok(None),
            Some(updated) => {
                let updated = updated.try_into()?;
                Ok(Some(updated))
            }
        }
    }

    async fn delete_price_component(
        &self,
        component_id: PriceComponentId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        PriceComponentRow::delete_by_id_and_tenant(&mut conn, component_id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }
}
