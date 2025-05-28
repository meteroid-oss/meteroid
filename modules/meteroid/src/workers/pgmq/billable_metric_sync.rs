use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::outbox::to_outbox_events;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::try_join_all;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::BillableMetric;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqMessage;
use std::sync::Arc;

pub(crate) struct BillableMetricSync {
    usage_client: Arc<dyn UsageClient>,
}

impl BillableMetricSync {
    pub fn new(usage_client: Arc<dyn UsageClient>) -> Self {
        Self { usage_client }
    }
}

#[async_trait::async_trait]
impl PgmqHandler for BillableMetricSync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let msg_id_to_out_evt = to_outbox_events(msgs).await?;

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(msg_id, event)| {
                let usage_client = self.usage_client.clone();

                tokio::spawn({
                    async move {
                        let tenant_id = event.tenant_id();

                        let metric = match event {
                            OutboxEvent::BillableMetricCreated(event) => event,
                            _ => {
                                return Err(PgmqError::HandleMessages)
                                    .attach_printable("Invalid event type");
                            }
                        };
                        let metric: BillableMetric = (*metric).into();
                        usage_client
                            .register_meter(tenant_id, &metric)
                            .await
                            .attach_printable("Failed to register meter")
                            .change_context(PgmqError::HandleMessages)?;
                        Ok::<MessageId, Report<PgmqError>>(msg_id)
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
