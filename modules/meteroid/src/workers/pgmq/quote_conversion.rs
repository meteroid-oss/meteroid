use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::ids::BaseId;
use common_domain::pgmq::MessageId;
use error_stack::ResultExt;
use meteroid_store::Services;
use meteroid_store::StoreResult;
use meteroid_store::domain::pgmq::{PgmqMessage, QuoteConversionRequestEvent};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) struct QuoteConversion {
    services: Arc<Services>,
}

impl QuoteConversion {
    pub fn new(services: Arc<Services>) -> Self {
        Self { services }
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(QuoteConversionRequestEvent, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<QuoteConversionRequestEvent> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for QuoteConversion {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let msg_id_to_out_evt = self.convert_to_events(msgs)?;

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(event, msg_id)| {
                let services = self.services.clone();

                tokio::spawn(async move {
                    let QuoteConversionRequestEvent::QuoteAccepted(quote_accepted) = event;

                    let tenant_id = quote_accepted.tenant_id;
                    let quote_id = quote_accepted.quote_id;

                    // Use system user for automated conversion
                    let created_by = Uuid::nil();

                    match services
                        .convert_quote_to_subscription(tenant_id, quote_id, created_by)
                        .await
                    {
                        Ok(result) => {
                            log::info!(
                                "Quote {} converted to subscription {} successfully",
                                quote_id.as_uuid(),
                                result.subscription.id.as_uuid()
                            );
                            Ok((msg_id, quote_id))
                        }
                        Err(e) => {
                            log::error!(
                                "Failed to convert quote {} to subscription: {:?}",
                                quote_id.as_uuid(),
                                e
                            );
                            Err(HandleResult::fail(msg_id, &e))
                        }
                    }
                })
            })
            .collect();

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();

        for task in tasks {
            match task.await {
                Ok(Ok((msg_id, _))) => succeeded.push(msg_id),
                Ok(Err((msg_id, error))) => failed.push((msg_id, error)),
                Err(e) => {
                    log::error!("Quote conversion task panicked: {e:?}");
                }
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}
