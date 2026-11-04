// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/kafka.rs

use backon::Retryable;
use rdkafka::{
    ClientConfig,
    admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
    config::FromClientConfig,
};
use std::sync::Arc;
use std::time::Duration;

pub const CONTAINER_NAME: &str = "redpandadata/redpanda";
pub const CONTAINER_VERSION: &str = "v23.3.2";

pub async fn create_topic(port: u16, topic: &str) -> anyhow::Result<()> {
    let mut admin_config = ClientConfig::new();
    let broker = format!("127.0.0.1:{port}");
    let num_partitions = 3;
    let num_replicas = 1;
    admin_config
        .set("client.id", "test-admin")
        .set("bootstrap.servers", &broker);

    let admin_client = AdminClient::from_config(&admin_config)?;
    let options = AdminOptions::default();
    let res = admin_client
        .create_topics(
            vec![&NewTopic::new(
                topic,
                num_partitions,
                TopicReplication::Fixed(num_replicas),
            )],
            &options,
        )
        .await?;
    for r in res {
        match r {
            Err((topic, err)) => {
                log::error!("Error creating topic {}: {}", &topic, err);
            }
            Ok(topic) => {
                log::info!("Created topic {}", topic);
            }
        }
    }
    Ok(())
}

/// The consumer commits an offset only once its batch is stored or dropped, so a group
/// caught up with the topic has handled every event sent so far.
pub async fn wait_until_consumed(kafka_port: u16, topic: &'static str) -> anyhow::Result<()> {
    use rdkafka::consumer::{BaseConsumer, Consumer};
    use rdkafka::{ClientConfig, Offset, TopicPartitionList};

    let consumer: Arc<BaseConsumer> = Arc::new(
        ClientConfig::new()
            .set("bootstrap.servers", format!("127.0.0.1:{kafka_port}"))
            .set("group.id", "clickhouse-events-raw")
            .create()?,
    );
    let timeout = Duration::from_secs(5);

    (|| {
        let consumer = consumer.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let metadata = consumer.fetch_metadata(Some(topic), timeout)?;
                let mut partitions = TopicPartitionList::new();
                for partition in metadata.topics()[0].partitions() {
                    partitions.add_partition(topic, partition.id());
                }
                let committed = consumer.committed_offsets(partitions.clone(), timeout)?;
                for partition in partitions.elements() {
                    let (_, high) =
                        consumer.fetch_watermarks(topic, partition.partition(), timeout)?;
                    let offset = committed
                        .find_partition(topic, partition.partition())
                        .map(|p| p.offset());
                    if high > 0 && offset != Some(Offset::Offset(high)) {
                        anyhow::bail!(
                            "partition {} at {offset:?} of {high}",
                            partition.partition()
                        );
                    }
                }
                Ok(())
            })
            .await?
        }
    })
    .retry(
        backon::ConstantBuilder::default()
            .with_delay(Duration::from_millis(200))
            .with_max_times(100),
    )
    .await
}
