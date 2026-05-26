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

    /// Insert a webhook event with idempotency on `(provider_config_id,
    /// provider_event_id)`. Returns `Ok(None)` when the provider redelivered
    /// an event we've already stored — the router uses that signal to ACK
    /// 200 without re-running the handler.
    pub async fn insert_webhook_in_event_idempotent(
        &self,
        event: WebhookInEventNew,
    ) -> StoreResult<Option<WebhookInEvent>> {
        let mut conn = self.services.store.get_conn().await?;

        let insertable: WebhookInEventRowNew = event.into();

        insertable
            .insert_or_skip_if_duplicate(&mut conn)
            .await
            .map(|opt| opt.map(Into::into))
            .map_err(Into::into)
    }
}
