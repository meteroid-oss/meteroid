use crate::store::Store;
use crate::StoreResult;
use error_stack::Report;

use crate::domain::price_components::{PriceComponent, PriceComponentNew};
use uuid::Uuid;

use crate::errors::StoreError;

#[async_trait::async_trait]
pub trait PriceComponentInterface {
    async fn list_price_components(
        &self,
        plan_version_id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<PriceComponent>>;

    async fn get_price_component_by_id(
        &self,
        tenant_id: Uuid,
        id: Uuid,
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
        tenant_id: Uuid,
        plan_version_id: Uuid,
    ) -> StoreResult<Option<PriceComponent>>;

    async fn delete_price_component(&self, component_id: Uuid, tenant_id: Uuid) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl PriceComponentInterface for Store {
    async fn list_price_components(
        &self,
        plan_version_id: Uuid,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<PriceComponent>> {
        let mut conn = self.get_conn().await?;

        let components = diesel_models::price_components::PriceComponent::list_by_plan_version_id(
            &mut conn,
            tenant_id,
            plan_version_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        components
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()
    }

    async fn get_price_component_by_id(
        &self,
        tenant_id: Uuid,
        price_component_id: Uuid,
    ) -> StoreResult<PriceComponent> {
        let mut conn = self.get_conn().await?;

        diesel_models::price_components::PriceComponent::get_by_id(
            &mut conn,
            tenant_id,
            price_component_id,
        )
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
        let inserted =
            diesel_models::price_components::PriceComponent::insert(&mut conn, price_component)
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
        let inserted = diesel_models::price_components::PriceComponent::insert_batch(
            &mut conn,
            price_components,
        )
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
        tenant_id: Uuid,
        plan_version_id: Uuid,
    ) -> StoreResult<Option<PriceComponent>> {
        let json_fee = serde_json::to_value(&price_component.fee).map_err(|e| {
            StoreError::SerdeError("Failed to serialize price component fee".to_string(), e)
        })?;

        let mut conn = self.get_conn().await?;
        let price_component: diesel_models::price_components::PriceComponent =
            diesel_models::price_components::PriceComponent {
                id: price_component.id,
                plan_version_id: plan_version_id,
                name: price_component.name,
                product_item_id: price_component.product_item_id,
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

    async fn delete_price_component(&self, component_id: Uuid, tenant_id: Uuid) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        diesel_models::price_components::PriceComponent::delete_by_id_and_tenant(
            &mut conn,
            component_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }
}
