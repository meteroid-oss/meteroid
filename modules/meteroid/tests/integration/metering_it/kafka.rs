// APACHE 2.0 license - Copyright 2021, The Tremor Team
// Adapted from https://github.com/tremor-rs/tremor-runtime/blob/main/src/connectors/tests/kafka.rs

use rdkafka::{
    admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
    config::FromClientConfig,
    ClientConfig,
};

pub(crate) const CONTAINER_NAME: &str = "redpandadata/redpanda";
pub(crate) const CONTAINER_VERSION: &str = "v23.3.2";

pub(crate) async fn create_topic(port: u16, topic: &str) -> anyhow::Result<()> {
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
