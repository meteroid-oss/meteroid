use crate::config::KafkaConnectionConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};

pub fn create_kafka_consumer(
    conn_config: &KafkaConnectionConfig,
    topic: &str,
    group_id: &str,
) -> StreamConsumer {
    let mut client_config = conn_config.to_client_config();

    // todo allow configs passed from outside
    let consumer: StreamConsumer = client_config
        .set("auto.offset.reset", "earliest")
        .set("group.id", group_id)
        .set("enable.partition.eof", "false")
        .set("session.timeout.ms", "6000")
        .set("enable.auto.commit", "false")
        .create()
        .expect("Failed to create Kafka consumer");

    consumer
        .subscribe(&[topic])
        .expect("Failed to subscribe to Kafka topic");

    log::info!(
        "Kafka consumer created and subscribed to topic '{}' with group ID '{}'",
        topic,
        group_id
    );

    consumer
}
