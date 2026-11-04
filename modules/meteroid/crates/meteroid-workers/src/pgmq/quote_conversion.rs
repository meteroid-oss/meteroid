use crate::pgmq::PgmqResult;
use crate::pgmq::outbox::{decode_each, quote_from_outbox};
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::actor::Actor;
use common_domain::ids::BaseId;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::{PgmqMessage, QuoteConversionRequestEvent};
use meteroid_store::{Services, StoreResult};
use std::sync::Arc;

pub(crate) struct QuoteConversion {
    services: Arc<Services>,
}

impl QuoteConversion {
    pub fn new(services: Arc<Services>) -> Self {
        Self { services }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for QuoteConversion {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = decode_each(msgs, |msg| {
            let event: StoreResult<OutboxEvent> = msg.try_into();
            event.map(quote_from_outbox)
        });

        let tasks: Vec<_> = decoded
            .events
            .into_iter()
            .map(|(msg_id, event)| {
                let services = self.services.clone();

                let task = tokio::spawn(async move {
                    let QuoteConversionRequestEvent::QuoteAccepted(quote_accepted) = event;

                    let tenant_id = quote_accepted.tenant_id;
                    let quote_id = quote_accepted.quote_id;

                    match services
                        .convert_quote_to_subscription(
                            Actor::Customer {
                                id: quote_accepted.customer_id,
                            },
                            tenant_id,
                            quote_id,
                        )
                        .await
                    {
                        Ok(result) => {
                            log::info!(
                                "Quote {} converted to subscription {} successfully",
                                quote_id.as_uuid(),
                                result.subscription.id.as_uuid()
                            );
                            Ok(quote_id)
                        }
                        Err(e) => {
                            log::error!(
                                "Failed to convert quote {} to subscription: {:?}",
                                quote_id.as_uuid(),
                                e
                            );
                            Err(e)
                        }
                    }
                });
                (msg_id, task)
            })
            .collect();

        let mut succeeded = Vec::new();
        let mut failed = decoded.failed;

        for (msg_id, task) in tasks {
            match task.await {
                Ok(Ok(_)) => succeeded.push(msg_id),
                Ok(Err(e)) => failed.push(HandleResult::fail(msg_id, &e)),
                Err(e) => {
                    log::error!("Quote conversion task panicked: {e:?}");
                    failed.push(HandleResult::fail(msg_id, &e));
                }
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
