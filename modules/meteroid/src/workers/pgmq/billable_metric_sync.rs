use crate::workers::pgmq::PgmqResult;
use crate::workers::pgmq::error::PgmqError;
use crate::workers::pgmq::processor::PgmqHandler;
use common_domain::pgmq::MessageId;
use error_stack::{Report, ResultExt};
use futures::future::try_join_all;
use meteroid_store::Store;
use meteroid_store::StoreResult;
use meteroid_store::clients::usage::UsageClient;
use meteroid_store::domain::BillableMetric;
use meteroid_store::domain::pgmq::{BillableMetricSyncRequestEvent, PgmqMessage};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use std::sync::Arc;

pub(crate) struct BillableMetricSync {
    usage_client: Arc<dyn UsageClient>,
    store: Store,
}

impl BillableMetricSync {
    pub fn new(usage_client: Arc<dyn UsageClient>, store: Store) -> Self {
        Self {
            usage_client,
            store,
        }
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
                let store = self.store.clone();

                tokio::spawn({
                    async move {
                        let tenant_id = event.tenant_id();

                        let BillableMetricSyncRequestEvent::BillableMetricCreated(metric) = event;
                        let metric: BillableMetric = (*metric).into();
                        let metric_id = metric.id;

                        match usage_client.register_meter(tenant_id, &metric).await {
                            Ok(_) => {
                                // Mark as synced successfully
                                let _ = store
                                    .mark_billable_metric_synced(metric_id, tenant_id, None)
                                    .await;
                                Ok::<MessageId, Report<PgmqError>>(msg_id)
                            }
                            Err(e) => {
                                // Mark sync error
                                let error_message = format!("{:?}", e);
                                let _ = store
                                    .mark_billable_metric_synced(
                                        metric_id,
                                        tenant_id,
                                        Some(error_message.clone()),
                                    )
                                    .await;
                                Err(e
                                    .attach("Failed to register meter")
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
