use crate::config::KafkaConfig;
use common_grpc::middleware::client::LayeredClientService;
use kafka::processor::run_message_processor;
use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;
use rdkafka::producer::FutureProducer;
use std::sync::Arc;

pub mod handler;

pub async fn run_raw_preprocessor(
    kafka_config: &KafkaConfig,
    internal_client: InternalServiceClient<LayeredClientService>,
) {
    let client_config = kafka_config.kafka_connection.to_client_config();

    let producer: FutureProducer = client_config
        .create()
        .expect("Failed to create Kafka producer");

    let handler = Arc::new(handler::PreprocessorHandler {
        producer,
        preprocessed_topic: kafka_config.kafka_preprocessed_topic.clone(),
        internal_client,
    });

    run_message_processor(
        &kafka_config.kafka_connection,
        &[&kafka_config.kafka_raw_topic],
        "meteroid-raw-preprocessor",
        handler,
    )
    .await
}
