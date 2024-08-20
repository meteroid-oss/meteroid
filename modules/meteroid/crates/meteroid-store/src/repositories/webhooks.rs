use crate::domain::webhooks::{
    WebhookInEvent, WebhookInEventNew, WebhookOutEndpoint, WebhookOutEndpointNew, WebhookOutEvent,
    WebhookOutEventNew,
};
use crate::domain::{OrderByRequest, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use diesel_models::webhooks::{
    WebhookInEventRowNew, WebhookOutEndpointRow, WebhookOutEventRow, WebhookOutEventRowNew,
};
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait WebhooksInterface {
    async fn insert_webhook_out_endpoint(
        &self,
        endpoint: WebhookOutEndpointNew,
    ) -> StoreResult<WebhookOutEndpoint>;

    async fn list_webhook_out_endpoints(
        &self,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<WebhookOutEndpoint>>;

    async fn insert_webhook_event(
        &self,
        endpoint: WebhookOutEventNew,
    ) -> StoreResult<WebhookOutEvent>;

    async fn list_webhook_out_events(
        &self,
        tenant_id: Uuid,
        endpoint_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<WebhookOutEvent>>;

    async fn insert_webhook_in_event(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<WebhookInEvent>;
}

#[async_trait::async_trait]
impl WebhooksInterface for Store {
    async fn insert_webhook_out_endpoint(
        &self,
        endpoint: WebhookOutEndpointNew,
    ) -> StoreResult<WebhookOutEndpoint> {
        let insertable = endpoint.to_row(&self.settings.crypt_key)?;

        let mut conn = self.get_conn().await?;

        let row = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        WebhookOutEndpoint::from_row(&self.settings.crypt_key, row)
    }

    async fn list_webhook_out_endpoints(
        &self,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<WebhookOutEndpoint>> {
        let mut conn = self.get_conn().await?;

        let vec_rows = WebhookOutEndpointRow::list_by_tenant_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        vec_rows
            .into_iter()
            .map(|row| WebhookOutEndpoint::from_row(&self.settings.crypt_key, row))
            .collect()
    }

    async fn insert_webhook_event(
        &self,
        endpoint: WebhookOutEventNew,
    ) -> StoreResult<WebhookOutEvent> {
        let mut conn = self.get_conn().await?;

        let insertable: WebhookOutEventRowNew = endpoint.into();

        let row = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(row.into())
    }

    async fn list_webhook_out_events(
        &self,
        tenant_id: Uuid,
        endpoint_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<WebhookOutEvent>> {
        let mut conn = self.get_conn().await?;

        let rows = WebhookOutEventRow::list_events(
            &mut conn,
            tenant_id,
            endpoint_id,
            pagination.into(),
            order_by.into(),
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<WebhookOutEvent> = PaginatedVec {
            items: rows.items.into_iter().map(|s| s.into()).collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn insert_webhook_in_event(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<WebhookInEvent> {
        let mut conn = self.get_conn().await?;

        let insertable: WebhookInEventRowNew = event.into();

        insertable
            .insert(&mut conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
