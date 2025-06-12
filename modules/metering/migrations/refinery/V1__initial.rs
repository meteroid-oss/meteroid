use crate::migrations::get_kafka_config;

pub fn migration() -> String {
    let cfg = get_kafka_config();

    format!(
        r#"
        CREATE TABLE IF NOT EXISTS meteroid.raw_events (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = MergeTree()
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, timestamp, code, customer_id);

        CREATE TABLE IF NOT EXISTS meteroid.raw_kafka_events (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = Kafka()
          SETTINGS
              kafka_broker_list = '{}',
              kafka_topic_list = '{}',
              kafka_group_name = 'clickhouse-events-raw',
              kafka_format = 'JSONEachRow';

         CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.raw_kafka_events_mv TO meteroid.raw_events AS
                SELECT * FROM meteroid.raw_kafka_events;
    "#,
        &cfg.kafka_internal_addr, &cfg.kafka_raw_topic
    )
}
