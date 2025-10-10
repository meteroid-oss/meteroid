use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::PgmqHandler;
use crate::workers::webhook_out::to_webhook_out;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::try_join_all;
use meteroid_store::Services;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqMessage;
use meteroid_store::domain::webhooks::WebhookOutCreateMessageResult;
use std::sync::Arc;

pub(crate) struct WebhookOut {
    pub services: Arc<Services>,
}

impl WebhookOut {
    pub fn new(services: Arc<Services>) -> Self {
        Self { services }
    }

    async fn handle_event(
        msg_id: MessageId,
        event: OutboxEvent,
        services: Arc<Services>,
    ) -> Result<MessageId, Report<PgmqError>> {
        let event_id = event.event_id();
        let tenant_id = event.tenant_id();
        let webhook_out = to_webhook_out(event).change_context(PgmqError::HandleMessages)?;

        if let Some(webhook_out) = webhook_out {
            let res = services
                .insert_webhook_message_out(tenant_id, webhook_out)
                .await
                .change_context(PgmqError::HandleMessages)?;

            match res {
                WebhookOutCreateMessageResult::Created(msg) => {
                    log::info!("Sent {} webhook {}", msg.event_type, msg.id);
                }
                WebhookOutCreateMessageResult::Conflict => {
                    log::warn!("Skipped webhook {event_id} as it already exists");
                }
                WebhookOutCreateMessageResult::NotFound => {
                    log::warn!(
                        "Skipped webhook {event_id} as the webhooks seem to not be configured for tenant {tenant_id}"
                    );
                }
                WebhookOutCreateMessageResult::SvixNotConfigured => {
                    log::warn!("Skipped webhook {event_id} as svix client not configured");
                }
            }
        }

        Ok::<MessageId, Report<PgmqError>>(msg_id)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for WebhookOut {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let msg_id_to_out_evt = to_outbox_events(msgs).await?;

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(msg_id, event)| {
                let services = self.services.clone();
                tokio::spawn(async move {
                    let event_type = event.event_type();
                    let res = Self::handle_event(msg_id, event, services).await;

                    if let Err(ref e) = res {
                        log::warn!(
                            "Failed to handle webhook_out {} event with msg_id={}: {:?}",
                            event_type,
                            msg_id.0,
                            e
                        );
                    }

                    res
                })
            })
            .collect();

        let results = try_join_all(tasks)
            .await
            .change_context(PgmqError::HandleMessages)?;

        let ids: Vec<_> = results.into_iter().filter_map(Result::ok).collect();

        Ok(ids)
    }
}
