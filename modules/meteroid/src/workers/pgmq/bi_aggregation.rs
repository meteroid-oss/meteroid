use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::ids::BaseId;
use common_domain::pgmq::MessageId;
use error_stack::ResultExt;
use futures::future::try_join_all;
use meteroid_store::Store;
use meteroid_store::StoreResult;
use meteroid_store::domain::pgmq::{BiAggregationEvent, PgmqMessage};
use meteroid_store::repositories::bi::{
    BiAggregationInterface, CreditNoteRevenueInput, InvoiceRevenueInput,
};
use std::sync::Arc;

pub(crate) struct BiAggregation {
    store: Arc<Store>,
}

impl BiAggregation {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(BiAggregationEvent, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<BiAggregationEvent> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for BiAggregation {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let msg_id_to_out_evt = self.convert_to_events(msgs)?;

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(event, msg_id)| {
                let store = self.store.clone();

                tokio::spawn({
                    async move {
                        let result = match event {
                            BiAggregationEvent::InvoiceFinalized(evt) => {
                                log::debug!(
                                    "Recording invoice revenue for tenant {} customer {} amount {}",
                                    evt.tenant_id.as_uuid(),
                                    evt.customer_id.as_uuid(),
                                    evt.amount_due
                                );

                                store
                                    .record_invoice_revenue(InvoiceRevenueInput {
                                        tenant_id: evt.tenant_id,
                                        customer_id: evt.customer_id,
                                        plan_version_id: evt.plan_version_id,
                                        currency: evt.currency,
                                        amount_cents: evt.amount_due,
                                        finalized_at: evt.finalized_at,
                                    })
                                    .await
                            }
                            BiAggregationEvent::CreditNoteFinalized(evt) => {
                                log::debug!(
                                    "Recording credit note revenue for tenant {} customer {} amount -{}",
                                    evt.tenant_id.as_uuid(),
                                    evt.customer_id.as_uuid(),
                                    evt.refunded_amount_cents
                                );

                                store
                                    .record_credit_note_revenue(CreditNoteRevenueInput {
                                        tenant_id: evt.tenant_id,
                                        customer_id: evt.customer_id,
                                        plan_version_id: evt.plan_version_id,
                                        currency: evt.currency,
                                        refunded_amount_cents: evt.refunded_amount_cents,
                                        finalized_at: evt.finalized_at,
                                    })
                                    .await
                            }
                        };

                        match result {
                            Ok(()) => {
                                log::debug!("BI aggregation event processed successfully");
                                Ok::<MessageId, error_stack::Report<PgmqError>>(msg_id)
                            }
                            Err(e) => {
                                log::error!("Failed to process BI aggregation event: {:?}", e);
                                Err(e
                                    .attach("Failed to process BI aggregation event")
                                    .change_context(PgmqError::HandleMessages))
                            }
                        }
                    }
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
