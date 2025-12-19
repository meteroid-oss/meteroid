use crate::StoreResult;
use crate::domain::webhooks::{WebhookInEvent, WebhookInEventNew};
use crate::services::ServicesEdge;
use diesel_models::webhooks::WebhookInEventRowNew;

#[allow(deprecated)]
impl ServicesEdge {
    pub async fn insert_webhook_in_event(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<WebhookInEvent> {
        let mut conn = self.services.store.get_conn().await?;

        let insertable: WebhookInEventRowNew = event.into();

        insertable
            .insert(&mut conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
