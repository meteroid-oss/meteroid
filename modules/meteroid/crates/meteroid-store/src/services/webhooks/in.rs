use crate::StoreResult;
use crate::domain::pgmq::{PgmqQueue, WebhookInProcessEvent};
use crate::domain::webhooks::{WebhookInEvent, WebhookInEventNew};
use crate::errors::StoreError;
use crate::repositories::pgmq::PgmqInterface;
use crate::services::ServicesEdge;
use common_domain::ids::TenantId;
use diesel_models::webhooks::{WebhookInEventRow, WebhookInEventRowNew};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;
use uuid::Uuid;

impl ServicesEdge {
    /// Persist the inbound webhook event and enqueue it for async processing,
    /// atomically in a single transaction.
    ///
    /// Returns `false` when the event was already ingested (deduped on the
    /// provider event id), in which case nothing is enqueued.
    pub async fn ingest_webhook_in_event(
        &self,
        event: WebhookInEventNew,
        tenant_id: TenantId,
    ) -> StoreResult<bool> {
        self.store
            .transaction(|conn| {
                async move {
                    let row_new: WebhookInEventRowNew = event.into();

                    let inserted = row_new
                        .insert_dedup(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let Some(row) = inserted else {
                        // Duplicate delivery — already ingested.
                        return Ok(false);
                    };

                    let msg = WebhookInProcessEvent::new(row.id, tenant_id);
                    self.store
                        .pgmq_send_batch_tx(conn, PgmqQueue::WebhookIn, vec![msg.try_into()?])
                        .await?;

                    Ok(true)
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn get_webhook_in_event(&self, event_uid: Uuid) -> StoreResult<WebhookInEvent> {
        let mut conn = self.store.get_conn().await?;

        WebhookInEventRow::get_by_id(&mut conn, event_uid)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    pub async fn mark_webhook_in_processed(&self, event_uid: Uuid) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        let processed_at = chrono::Utc::now().naive_utc();

        WebhookInEventRow::mark_processed(&mut conn, event_uid, processed_at)
            .await
            .map_err(Into::into)
    }

    pub async fn mark_webhook_in_failed(&self, event_uid: Uuid, error: String) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;

        WebhookInEventRow::mark_failed(&mut conn, event_uid, error)
            .await
            .map_err(Into::into)
    }
}
