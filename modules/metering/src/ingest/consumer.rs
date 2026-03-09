use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::ingest::domain::{RawEvent, RawEventRow};
use clickhouse::Client;
use kafka::consumer::create_kafka_consumer;
use rdkafka::consumer::{CommitMode, Consumer};
use rdkafka::message::Message;
use std::time::Duration;
use tokio::time;

const CONSUMER_GROUP_ID: &str = "clickhouse-events-raw";
const RAW_EVENTS_TABLE: &str = "raw_events";
const BATCH_MAX_ROWS: u64 = 2000;
const BATCH_PERIOD: Duration = Duration::from_millis(500);
const RESTART_DELAY: Duration = Duration::from_secs(5);

pub async fn run(kafka_config: &KafkaConfig, clickhouse_config: &ClickhouseConfig) {
    loop {
        if let Err(e) = run_inner(kafka_config, clickhouse_config).await {
            log::error!(
                "Kafka consumer error, restarting in {}s: {e:?}",
                RESTART_DELAY.as_secs()
            );
            time::sleep(RESTART_DELAY).await;
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ConsumerError {
    #[error("ClickHouse error: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),
    #[error("Kafka error: {0}")]
    Kafka(#[from] rdkafka::error::KafkaError),
}

async fn run_inner(
    kafka_config: &KafkaConfig,
    clickhouse_config: &ClickhouseConfig,
) -> Result<(), ConsumerError> {
    let consumer = create_kafka_consumer(
        &kafka_config.kafka_connection,
        &[&kafka_config.kafka_raw_topic],
        CONSUMER_GROUP_ID,
    );

    let client = Client::default()
        .with_url(&clickhouse_config.http_address)
        .with_user(&clickhouse_config.username)
        .with_password(&clickhouse_config.password)
        .with_database(&clickhouse_config.database);

    let mut inserter = client
        .inserter::<RawEventRow>(RAW_EVENTS_TABLE)
        .with_max_rows(BATCH_MAX_ROWS)
        .with_period(Some(BATCH_PERIOD));

    let mut interval = time::interval(BATCH_PERIOD);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            msg_result = consumer.recv() => {
                let msg = msg_result?;

                if let Some(payload) = msg.payload() {
                    match serde_json::from_slice::<RawEvent>(payload) {
                        Ok(event) => inserter.write(&RawEventRow::from(event)).await?,
                        Err(e) => log::warn!("Failed to deserialize event at partition={} and offset={}, skipping: {e:?}", msg.partition(),  msg.offset()),
                    }
                }

                let quantities = inserter.commit().await?;
                if quantities.rows > 0 {
                    consumer.commit_consumer_state(CommitMode::Async)?;
                    log::info!("Flushed {} raw events to ClickHouse", quantities.rows);
                }
            }

            _ = interval.tick() => {
                let quantities = inserter.commit().await?;
                if quantities.rows > 0 {
                    consumer.commit_consumer_state(CommitMode::Async)?;
                    log::info!("Flushed {} raw events to ClickHouse (periodic)", quantities.rows);
                }
            }
        }
    }
}
