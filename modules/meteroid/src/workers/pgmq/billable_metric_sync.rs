use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::try_join_all;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::BillableMetric;
use meteroid_store::domain::pgmq::{BillableMetricSyncRequestEvent, PgmqMessage};
use std::sync::Arc;

pub(crate) struct BillableMetricSync {
    usage_client: Arc<dyn UsageClient>,
}

impl BillableMetricSync {
    pub fn new(usage_client: Arc<dyn UsageClient>) -> Self {
        Self { usage_client }
    }

    fn convert_to_events(
        &self,
        msgs: &[PgmqMessage],
    ) -> PgmqResult<Vec<(BillableMetricSyncRequestEvent, MessageId)>> {
        msgs.iter()
            .map(|msg| {
                let evt: StoreResult<BillableMetricSyncRequestEvent> = msg.try_into();
                evt.map(|evt| (evt, msg.msg_id))
            })
            .collect::<StoreResult<Vec<_>>>()
            .change_context(PgmqError::HandleMessages)
    }
}

#[async_trait::async_trait]
impl PgmqHandler for BillableMetricSync {
    async fn handle(&self, msgs: &[PgmqMessage]) -> PgmqResult<Vec<MessageId>> {
        let msg_id_to_out_evt = self.convert_to_events(msgs)?;

        let tasks: Vec<_> = msg_id_to_out_evt
            .into_iter()
            .map(|(event, msg_id)| {
                let usage_client = self.usage_client.clone();

                tokio::spawn({
                    async move {
                        let tenant_id = event.tenant_id();

                        let BillableMetricSyncRequestEvent::BillableMetricCreated(metric) = event;
                        let metric: BillableMetric = (*metric).into();
                        usage_client
                            .register_meter(tenant_id, &metric)
                            .await
                            .attach("Failed to register meter")
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
