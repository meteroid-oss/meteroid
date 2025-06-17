use crate::migrations::get_kafka_config;

pub fn migration() -> String {
    let cfg = get_kafka_config();

    format!(
        r#"
        CREATE TABLE IF NOT EXISTS meteroid.preprocessed_events (
            id String,
            code String,
            billable_metric_id String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            preprocessed_at DateTime64(9, 'UTC'),
            properties Map(String, String),
            value Nullable(Decimal(38, 26)),
            distinct_on Nullable(String),
            group_by_dim1 Nullable(String),
            group_by_dim2 Nullable(String)
        ) ENGINE = ReplacingMergeTree(timestamp)
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, code, billable_metric_id, customer_id, toDate(timestamp), timestamp, id);

        CREATE TABLE IF NOT EXISTS meteroid.preprocessed_kafka_events (
            id String,
            code String,
            billable_metric_id String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            preprocessed_at DateTime64(9, 'UTC'),
            properties Map(String, String),
            value Nullable(Decimal(38, 26)),
            distinct_on Nullable(String),
            group_by_dim1 Nullable(String),
            group_by_dim2 Nullable(String)
        ) ENGINE = Kafka()
          SETTINGS
              kafka_broker_list = '{}',
              kafka_topic_list = '{}',
              kafka_group_name = 'clickhouse-events-preprocessed',
              kafka_format = 'JSONEachRow';

         CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.preprocessed_kafka_events_mv TO meteroid.preprocessed_events AS
                SELECT * FROM meteroid.preprocessed_kafka_events;
    "#,
        &cfg.kafka_internal_addr, &cfg.kafka_preprocessed_topic
    )
}
