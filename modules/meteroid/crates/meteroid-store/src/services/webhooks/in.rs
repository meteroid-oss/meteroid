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

    /// Look up whether an inbound event has already been recorded for a
    /// connector. A recorded row means we already processed (or permanently
    /// rejected) the event, so the webhook router skips re-handling it.
    pub async fn find_webhook_in_event(
        &self,
        provider_config_id: uuid::Uuid,
        provider_event_id: &str,
    ) -> StoreResult<Option<WebhookInEvent>> {
        use diesel_models::webhooks::WebhookInEventRow;
        let mut conn = self.services.store.get_conn().await?;

        WebhookInEventRow::find_by_provider_event(&mut conn, provider_config_id, provider_event_id)
            .await
            .map(|opt| opt.map(Into::into))
            .map_err(Into::into)
    }
}
