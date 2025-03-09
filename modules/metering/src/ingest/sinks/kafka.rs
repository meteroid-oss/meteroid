use crate::config::KafkaConfig;
use crate::ingest::domain::ProcessedEvent;
use crate::ingest::errors::IngestError;
use crate::ingest::metrics::{INGEST_BATCH_SIZE, INGESTED_EVENTS_TOTAL};
use crate::ingest::sinks::{FailedRecord, Sink};
use opentelemetry::KeyValue;
use rdkafka::error::{KafkaError, RDKafkaErrorCode};
use rdkafka::producer::{DeliveryFuture, FutureProducer, FutureRecord, Producer};
use rdkafka::util::Timeout;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tracing::instrument;
use tracing::log::{error, info};

#[derive(Clone)]
pub struct KafkaSink {
    producer: FutureProducer,
    topic: String,
}

// TODO check https://clickhouse.com/docs/en/integrations/kafka/kafka-table-engine#tuning-performance
// (although in high throughput we will not use the Kafka table engine)
impl KafkaSink {
    pub fn new(config: &KafkaConfig) -> Result<KafkaSink, KafkaError> {
        info!(
            "connecting to Kafka brokers at {:?}...",
            config.kafka_connection.bootstrap_servers
        );

        let client_config = config.to_client_config();

        let producer: FutureProducer = client_config.create()?;

        // Ping the cluster to make sure we can reach brokers, fail after 10 seconds
        _ = producer.client().fetch_metadata(
            Some("__consumer_offsets"),
            Timeout::After(Duration::new(10, 0)),
        )?;

        info!("connected to Kafka brokers");

        Ok(KafkaSink {
            producer,
            topic: config.kafka_topic.clone(),
        })
    }

    pub fn _flush(&self) -> Result<(), KafkaError> {
        // TODO: hook it up on shutdown
        self.producer.flush(Duration::new(30, 0))
    }

    async fn kafka_send(
        producer: FutureProducer,
        topic: String,
        event: &ProcessedEvent,
    ) -> Result<DeliveryFuture, IngestError> {
        // TODO proto
        let payload = serde_json::to_string(&event).map_err(|e| {
            error!("failed to serialize event: {}", e);
            IngestError::NonRetryableSinkError
        })?;

        match producer.send_result(FutureRecord {
            topic: topic.as_str(),
            payload: Some(&payload),
            partition: None,
            key: Some(event.key().as_str()),
            timestamp: None,
            headers: None,
        }) {
            Ok(ack) => Ok(ack),
            Err((e, _)) => match e.rdkafka_error_code() {
                Some(RDKafkaErrorCode::MessageSizeTooLarge) => {
                    // report_dropped_events("kafka_message_size", 1);
                    Err(IngestError::EventTooBig)
                }
                _ => {
                    // report_dropped_events("kafka_write_error", 1);
                    error!("failed to produce event: {}", e);
                    Err(IngestError::RetryableSinkError)
                }
            },
        }
    }

    async fn process_ack(
        delivery: DeliveryFuture,
        attributes: &[KeyValue],
    ) -> Result<(), IngestError> {
        match delivery.await {
            Err(_) => {
                // Cancelled due to timeout while retrying
                error!("failed to produce to Kafka before write timeout");
                Err(IngestError::RetryableSinkError)
            }
            Ok(Err((KafkaError::MessageProduction(RDKafkaErrorCode::MessageSizeTooLarge), _))) => {
                // Rejected by broker due to message size
                Err(IngestError::EventTooBig)
            }
            Ok(Err((err, _))) => {
                error!("failed to produce to Kafka: {}", err);
                Err(IngestError::RetryableSinkError)
            }
            Ok(Ok(_)) => {
                INGESTED_EVENTS_TOTAL.add(1, attributes);
                Ok(())
            }
        }
    }
}

#[async_trait::async_trait]
impl Sink for KafkaSink {
    #[instrument(skip_all)]
    async fn send(
        &self,
        events: Vec<ProcessedEvent>,
        attributes: &[KeyValue],
    ) -> Result<Vec<FailedRecord>, IngestError> {
        let mut set = JoinSet::new();
        let failed_events = Arc::new(Mutex::new(Vec::new()));
        let batch_size = events.len();
        let attributes_arc = Arc::new(attributes.to_vec());

        for event in events {
            let producer = self.producer.clone();
            let topic = self.topic.clone();
            // or sequentially ?
            // let ack = Self::kafka_send(producer, topic, event).await?;
            // set.spawn(Self::process_ack(ack, attributes));
            let attributes = attributes_arc.clone();

            set.spawn(async move {
                match Self::kafka_send(producer, topic, &event).await {
                    Ok(ack) => {
                        if let Err(error) = Self::process_ack(ack, &attributes).await {
                            vec![FailedRecord { event, error }]
                        } else {
                            vec![]
                        }
                    }
                    Err(error) => {
                        vec![FailedRecord { event, error }]
                    }
                }
            });
        }

        while let Some(res) = set.join_next().await {
            match res {
                Ok(mut local_failed_events) => {
                    if !local_failed_events.is_empty() {
                        let mut shared_failed_events = failed_events.lock().await;
                        shared_failed_events.append(&mut local_failed_events);
                    }
                }
                Err(err) => {
                    set.abort_all();
                    error!("Join error while waiting on Kafka ACK: {:?}", err);
                    return Err(IngestError::RetryableSinkError);
                }
            }
        }

        let failed_events = Arc::try_unwrap(failed_events)
            .map_err(|_| IngestError::RetryableSinkError)?
            .into_inner();

        let attributes = attributes_arc.clone();
        INGEST_BATCH_SIZE.record(batch_size as u64, &attributes);

        Ok(failed_events)
    }
}

#[cfg(test)]
mod tests {
    use crate::config;
    use crate::ingest::domain::ProcessedEvent;
    use crate::ingest::errors::IngestError;
    use crate::ingest::sinks::Sink;
    use crate::ingest::sinks::kafka::KafkaSink;
    use kafka::config::KafkaConnectionConfig;
    use rand::Rng;
    use rand::distr::Alphanumeric;
    use rdkafka::mocking::MockCluster;
    use rdkafka::producer::DefaultProducerContext;
    use rdkafka::types::{RDKafkaApiKey, RDKafkaRespErr};
    use std::collections::HashMap;

    async fn start_on_mocked_sink() -> (MockCluster<'static, DefaultProducerContext>, KafkaSink) {
        let cluster = MockCluster::new(1).expect("failed to create mock brokers");
        let config = config::KafkaConfig {
            kafka_connection: KafkaConnectionConfig {
                bootstrap_servers: Some(cluster.bootstrap_servers()),
                security_protocol: None,
                sasl_mechanism: None,
                sasl_username: None,
                sasl_password: None,
            },
            kafka_internal_addr: cluster.bootstrap_servers(),
            kafka_producer_linger_ms: 0,
            kafka_producer_queue_mib: 50,
            kafka_message_timeout_ms: 500,
            kafka_compression_codec: "none".to_string(),
            kafka_topic: "ingest_events".to_string(),
        };
        let sink = KafkaSink::new(&config).expect("failed to create sink");
        (cluster, sink)
    }

    #[tokio::test]
    async fn kafka_sink_error_handling() {
        // Uses a mocked Kafka broker that allows injecting write errors, to check error handling.
        // We test different cases in a single test to amortize the startup cost of the producer.

        let (cluster, sink) = start_on_mocked_sink().await;
        let event: ProcessedEvent = ProcessedEvent {
            event_id: "eventid".to_string(),
            event_name: "eventname".to_string(),
            customer_id: "customerid".to_string(),
            tenant_id: "tenantid".to_string(),
            event_timestamp: chrono::Utc::now().naive_utc(),
            properties: HashMap::from([("key".to_string(), "value".to_string())]),
        };

        let attributes = vec![];
        // Wait for producer to be healthy, to keep kafka_message_timeout_ms short and tests faster
        for _ in 0..20 {
            if sink.send(vec![event.clone()], &attributes).await.is_ok() {
                break;
            }
        }

        // Send events to confirm happy path
        sink.send(vec![event.clone()], &attributes)
            .await
            .expect("failed to send one initial event");
        sink.send(
            vec![event.clone(), event.clone(), event.clone(), event.clone()],
            &attributes,
        )
        .await
        .expect("failed to send initial event batch");

        // Producer should reject a 2MB message, twice the default `message.max.bytes`
        let big_data = rand::rng()
            .sample_iter(Alphanumeric)
            .take(2_000_000)
            .map(char::from)
            .collect::<String>();
        let big_event: ProcessedEvent = ProcessedEvent {
            event_id: "eventid".to_string(),
            event_name: "eventname".to_string(),
            customer_id: "customerid".to_string(),
            tenant_id: "tenantid".to_string(),
            event_timestamp: chrono::Utc::now().naive_utc(),
            properties: HashMap::from([("key".to_string(), big_data.to_string())]),
        };

        async fn check_error(sink: &KafkaSink, input: Vec<ProcessedEvent>, error: IngestError) {
            match sink.send(input, &[]).await {
                Err(err) => panic!("Sink failed the whole batch with error {}", err),
                Ok(vec) => match vec.first().map(|r| &r.error) {
                    Some(err) => {
                        if err != &error {
                            panic!("wrong error, expected: '{}', got '{}'", error, err)
                        }
                    }
                    None => panic!("should have errored"),
                },
            };
        }

        check_error(&sink, vec![big_event.clone()], IngestError::EventTooBig).await;

        // Simulate unretriable errors
        cluster.clear_request_errors(RDKafkaApiKey::Produce);
        let err = [RDKafkaRespErr::RD_KAFKA_RESP_ERR_MSG_SIZE_TOO_LARGE; 1];
        cluster.request_errors(RDKafkaApiKey::Produce, &err);
        check_error(&sink, vec![event.clone()], IngestError::EventTooBig).await;

        // Simulate transient errors, messages should go through OK
        cluster.clear_request_errors(RDKafkaApiKey::Produce);
        let err = [RDKafkaRespErr::RD_KAFKA_RESP_ERR_BROKER_NOT_AVAILABLE; 2];
        cluster.request_errors(RDKafkaApiKey::Produce, &err);
        sink.send(vec![event.clone()], &attributes)
            .await
            .expect("failed to send one event after recovery");

        // Timeout on a sustained transient error
        cluster.clear_request_errors(RDKafkaApiKey::Produce);
        let err = [RDKafkaRespErr::RD_KAFKA_RESP_ERR_BROKER_NOT_AVAILABLE; 50];
        cluster.request_errors(RDKafkaApiKey::Produce, &err);
        check_error(&sink, vec![event.clone()], IngestError::RetryableSinkError).await;
    }
}
