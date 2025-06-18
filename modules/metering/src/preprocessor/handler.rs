use crate::ingest::domain::{PreprocessedEvent, RawEvent};
use async_trait::async_trait;
use cached::proc_macro::cached;
use common_grpc::middleware::client::LayeredClientService;
use kafka::processor::MessageHandler;
use meteroid_grpc::meteroid::api::billablemetrics::v1::BillableMetric;
use meteroid_grpc::meteroid::api::billablemetrics::v1::aggregation::AggregationType;
use meteroid_grpc::meteroid::api::billablemetrics::v1::segmentation_matrix::Matrix;
use meteroid_grpc::meteroid::internal::v1::ListBillableMetricsRequest;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use rdkafka::Message;
use rdkafka::message::BorrowedMessage;
use rdkafka::producer::{FutureProducer, FutureRecord};
use tokio::task::JoinSet;
use tonic::Status;

#[async_trait]
impl MessageHandler for PreprocessorHandler {
    async fn handle(
        &self,
        message: &BorrowedMessage<'_>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(payload) = message.payload() {
            let raw: RawEvent = serde_json::from_slice(payload)?;
            self.preprocess_raw(raw).await?;
        } else {
            log::warn!("Received kafka message with no payload");
        }
        Ok(())
    }
}

pub struct PreprocessorHandler {
    pub producer: FutureProducer,
    pub preprocessed_topic: String,
    pub internal_client: InternalServiceClient<LayeredClientService>,
}

impl PreprocessorHandler {
    async fn preprocess_raw(&self, raw_event: RawEvent) -> Result<(), Box<dyn std::error::Error>> {
        let metrics = list_metrics_cached(
            &self.internal_client,
            raw_event.tenant_id.to_string(),
            raw_event.code.to_string(),
        )
        .await?;

        if metrics.is_empty() {
            log::warn!(
                "No billable metrics found for tenant_id: {}, code: {}, skipping event",
                raw_event.tenant_id,
                raw_event.code
            );
            return Ok(());
        }

        let mut set = JoinSet::new();

        for metric in metrics {
            let preprocessed_event = Self::convert_to_preprocessed(&raw_event, &metric);
            let as_json = serde_json::to_string(&preprocessed_event)?;

            let kafka_key = preprocessed_event.key();
            let record = FutureRecord::to(self.preprocessed_topic.as_str())
                .payload(as_json.as_str())
                .key(kafka_key.as_str());

            let delivery_future = self.producer.send_result(record).map_err(|(e, _)| e)?;

            set.spawn(async move {
                match delivery_future.await {
                    Ok(delivery) => {
                        log::debug!("Successfully sent preprocessed event: {:?}", delivery);
                    }
                    Err(e) => {
                        log::error!("Failed to send preprocessed event: {}", e);
                    }
                }
            });
        }

        set.join_all().await;

        Ok(())
    }

    fn convert_to_preprocessed(raw: &RawEvent, metric: &BillableMetric) -> PreprocessedEvent {
        let (dim1_key, dim2_key) = metric
            .segmentation_matrix
            .as_ref()
            .and_then(|m| m.matrix.as_ref())
            .map(|m| match m {
                Matrix::Single(single) => (single.dimension.as_ref().map(|x| x.key.clone()), None),
                Matrix::Double(double) => (
                    double.dimension1.as_ref().map(|x| x.key.clone()),
                    double.dimension2.as_ref().map(|x| x.key.clone()),
                ),
                Matrix::Linked(linked) => (
                    Some(linked.dimension_key.clone()),
                    Some(linked.linked_dimension_key.clone()),
                ),
            })
            .unwrap_or_default();

        let aggregation_key = metric
            .aggregation
            .as_ref()
            .and_then(|x| x.aggregation_key.clone());

        let (distinct_on, value) = metric
            .aggregation
            .as_ref()
            .map(|x| x.aggregation_type())
            .map(|agg_type| {
                if agg_type == AggregationType::CountDistinct {
                    let distinct_on = aggregation_key
                        .as_ref()
                        .and_then(|key| raw.properties.get(key).cloned());

                    (distinct_on, None)
                } else {
                    let value = aggregation_key.as_ref().and_then(|key| {
                        raw.properties
                            .get(key)
                            .and_then(|v| v.parse::<rust_decimal::Decimal>().ok())
                    });
                    (None, value)
                }
            })
            .unwrap_or_default();

        let group_by_dim1 = dim1_key
            .as_ref()
            .and_then(|key| raw.properties.get(key).cloned());

        let group_by_dim2 = dim2_key
            .as_ref()
            .and_then(|key| raw.properties.get(key).cloned());

        PreprocessedEvent {
            id: raw.id.to_owned(),
            tenant_id: raw.tenant_id.to_owned(),
            code: raw.code.to_owned(),
            billable_metric_id: metric.id.to_owned(),
            customer_id: raw.customer_id.to_owned(),
            timestamp: raw.timestamp.to_owned(),
            preprocessed_at: chrono::Utc::now().naive_utc(),
            properties: raw.properties.clone(),
            value,
            distinct_on,
            group_by_dim1,
            group_by_dim2,
        }
    }
}

#[cached(
    result = true,
    size = 100,
    time = 120, // 2 min
    key = "(String, String)",
    convert = r#"{ (tenant_id.clone(), code.clone()) }"#,
    sync_writes = "default"
)]
async fn list_metrics_cached(
    internal_client: &InternalServiceClient<LayeredClientService>,
    tenant_id: String,
    code: String,
) -> Result<Vec<BillableMetric>, Status> {
    let metrics = internal_client
        .clone()
        .list_billable_metrics(ListBillableMetricsRequest { tenant_id, code })
        .await?
        .into_inner()
        .items;

    Ok(metrics)
}
