use crate::pgmq::PgmqResult;
use crate::pgmq::outbox::decode_each;
use crate::pgmq::processor::{HandleResult, PgmqHandler};
use common_domain::pgmq::{MessageId, ReadCt};
use meteroid_store::domain::pgmq::{PaymentRequestEvent, PgmqMessage};
use meteroid_store::errors::StoreError;
use meteroid_store::{Services, StoreResult};
use std::sync::Arc;

/// Reads before a payment request dead-letters; the last failed read ends automatic collection.
/// With the processor's 15 s doubling backoff, 6 rides out a ~15 min provider outage.
pub(crate) const PAYMENT_REQUEST_MAX_READS: ReadCt = ReadCt(6);

#[derive(Clone)]
pub struct PaymentRequest {
    services: Arc<Services>,
}

impl PaymentRequest {
    pub(crate) fn new(services: Arc<Services>) -> Self {
        Self { services }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for PaymentRequest {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<HandleResult> {
        let decoded = decode_each(msgs, |msg| {
            let evt: StoreResult<PaymentRequestEvent> = msg.try_into();
            evt.map(|evt| Some((evt, msg.read_ct)))
        });

        let mut succeeded = Vec::new();
        let mut failed = decoded.failed;

        for (msg_id, (event, read_ct)) in decoded.events {
            match self.process_payment_request(msg_id, event, read_ct).await {
                Ok(()) => succeeded.push(msg_id),
                Err(failure) => failed.push(failure),
            }
        }

        Ok(HandleResult { succeeded, failed })
    }
}

impl PaymentRequest {
    #[tracing::instrument(skip_all, fields(
        tenant_id = %event.tenant_id,
        invoice_id = %event.invoice_id,
    ))]
    async fn process_payment_request(
        &self,
        msg_id: MessageId,
        event: PaymentRequestEvent,
        read_ct: ReadCt,
    ) -> Result<(), (MessageId, String)> {
        log::info!(
            "Processing payment request for invoice {} with payment method {}",
            event.invoice_id,
            event.payment_method_id
        );

        match self
            .services
            .complete_invoice_payment_automatic(
                event.tenant_id,
                event.invoice_id,
                event.payment_method_id,
                // Stable seed so a redelivery after a post-charge rollback reuses the
                // provider key and dedupes; `None` only for pre-seed legacy messages.
                event.idempotency_key.clone(),
            )
            .await
        {
            Ok((transaction, _next_action)) => {
                log::info!(
                    "Payment processed successfully for invoice {}: transaction {} with status {:?}",
                    event.invoice_id,
                    transaction.id,
                    transaction.status
                );
                Ok(())
            }
            // The connector was disabled or deleted after the charge was queued: retrying
            // can't help. The customer gets the invoice with a payment link instead.
            Err(err) if matches!(err.current_context(), StoreError::ConnectorNotActive(_)) => {
                log::warn!(
                    "Auto-charge of invoice {} skipped: {}",
                    event.invoice_id,
                    err.current_context()
                );
                match self
                    .services
                    .fall_back_to_manual_payment(event.tenant_id, event.invoice_id)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(fallback_err) => Err(HandleResult::fail(msg_id, &fallback_err)),
                }
            }
            Err(err) => {
                log::error!(
                    "Failed to process payment for invoice {}: {:?}",
                    event.invoice_id,
                    err
                );
                // The processor dead-letters this message: nothing will charge the invoice.
                if read_ct > PAYMENT_REQUEST_MAX_READS
                    && let Err(hand_over_err) = self
                        .services
                        .hand_over_to_reminders(event.tenant_id, event.invoice_id)
                        .await
                {
                    log::error!(
                        "Invoice {} not handed over to payment reminders: {:?}",
                        event.invoice_id,
                        hand_over_err
                    );
                }
                Err(HandleResult::fail(msg_id, &err))
            }
        }
    }
}
