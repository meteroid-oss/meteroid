use crate::domain::webhooks::{WebhookOutEndpoint, WebhookOutEndpointNew, WebhookOutEvent};
use crate::domain::{OrderByRequest, PaginatedVec, PaginationRequest};
use crate::errors::StoreError;
use crate::{Store, StoreResult};
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

    async fn list_webhook_out_events(
        &self,
        tenant_id: Uuid,
        endpoint_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<WebhookOutEvent>>;
}

#[async_trait::async_trait]
impl WebhooksInterface for Store {
    async fn insert_webhook_out_endpoint(
        &self,
        endpoint: WebhookOutEndpointNew,
    ) -> StoreResult<WebhookOutEndpoint> {
        let insertable = endpoint.to_row(&self.crypt_key)?;

        let mut conn = self.get_conn().await?;

        let row = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        WebhookOutEndpoint::from_row(&self.crypt_key, row)
    }

    async fn list_webhook_out_endpoints(
        &self,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<WebhookOutEndpoint>> {
        let mut conn = self.get_conn().await?;

        let vec_rows =
            diesel_models::webhooks::WebhookOutEndpoint::list_by_tenant_id(&mut conn, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        vec_rows
            .into_iter()
            .map(|row| WebhookOutEndpoint::from_row(&self.crypt_key, row))
            .collect()
    }

    async fn list_webhook_out_events(
        &self,
        tenant_id: Uuid,
        endpoint_id: Uuid,
        pagination: PaginationRequest,
        order_by: OrderByRequest,
    ) -> StoreResult<PaginatedVec<WebhookOutEvent>> {
        let mut conn = self.get_conn().await?;

        let rows = diesel_models::webhooks::WebhookOutEvent::list_events(
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
}
