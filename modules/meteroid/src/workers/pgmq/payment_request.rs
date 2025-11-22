use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::pgmq::MessageId;
use error_stack::ResultExt;
use meteroid_store::domain::pgmq::{PaymentRequestEvent, PgmqMessage};
use meteroid_store::{Services, StoreResult};
use std::sync::Arc;

#[derive(Clone)]
pub struct PaymentRequest {
    services: Arc<Services>,
}

impl PaymentRequest {
    pub(crate) fn new( services: Arc<Services>) -> Self {
        Self {  services }
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(PaymentRequestEvent, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<PaymentRequestEvent> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for PaymentRequest {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let events = self.convert_to_events(msgs)?;

        let mut succeeded = Vec::new();

        for (event, msg_id) in events {
            log::info!(
                "Processing payment request for invoice {} with payment method {}",
                event.invoice_id,
                event.payment_method_id
            );

            match self
                .services
                .complete_invoice_payment(
                    event.tenant_id,
                    event.invoice_id,
                    event.payment_method_id,
                )
                .await
            {
                Ok(transaction) => {
                    log::info!(
                        "Payment processed successfully for invoice {}: transaction {} with status {:?}",
                        event.invoice_id,
                        transaction.id,
                        transaction.status
                    );
                    succeeded.push(msg_id);
                }
                Err(err) => {
                    log::error!(
                        "Failed to process payment for invoice {}: {:?}",
                        event.invoice_id,
                        err
                    );
                    // Don't add to succeeded - let PGMQ retry
                }
            }
        }

        Ok(succeeded)
    }
}
