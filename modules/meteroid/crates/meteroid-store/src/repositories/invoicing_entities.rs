use error_stack::Report;
use serde::{Deserialize, Serialize};
use tracing_log::log;
use uuid::Uuid;

use common_eventbus::Event;
use diesel_models::invoicing_entities::InvoicingEntityRow;

use crate::domain::invoicing_entities::InvoicingEntity;
use crate::errors::StoreError;
use crate::store::Store;
use crate::StoreResult;


#[async_trait::async_trait]
pub trait InvoicingEntityInterface {
    async fn list_invoicing_entities(
        &self,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<InvoicingEntity>>;

    async fn get_invoicing_entity(
        &self,
        tenant_id: Uuid,
        invoicing_id_or_default: Option<Uuid>,
    ) -> StoreResult<InvoicingEntity>;

    async fn create_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntity,
    ) -> StoreResult<InvoicingEntity>;
}

#[async_trait::async_trait]
impl InvoicingEntityInterface for Store {
    async fn list_invoicing_entities(&self, tenant_id: Uuid) -> StoreResult<Vec<InvoicingEntity>> {
        let mut conn = self.get_conn().await?;

        let invoicing_entities = InvoicingEntityRow::list_by_tenant_id(&mut conn, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|v| v.into())
            .collect();

        Ok(invoicing_entities)
    }

    async fn get_invoicing_entity(&self, tenant_id: Uuid, invoicing_id_or_default: Option<Uuid>) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let invoicing_entity = match invoicing_id_or_default {
            Some(invoicing_id) => InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant_id(&mut conn, &invoicing_id, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into(),
            None => InvoicingEntityRow::get_default_invoicing_entity_for_tenant(&mut conn, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into(),
        };

        Ok(invoicing_entity)
    }

    async fn create_invoicing_entity(&self, invoicing_entity: InvoicingEntity) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let invoicing_entity_row: InvoicingEntityRow = invoicing_entity.into();

        let invoicing_entity_row = invoicing_entity_row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(invoicing_entity_row.into())
    }
}