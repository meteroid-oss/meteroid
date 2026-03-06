use crate::migrations::{get_clickhouse_config, get_kafka_config, build_kafka_settings};

pub fn migration() -> String {
    let kafka_cfg = get_kafka_config();
    let clickhouse_cfg = get_clickhouse_config();

    let raw_kafka_settings = build_kafka_settings(
        &kafka_cfg.kafka_internal_addr,
        &kafka_cfg.kafka_raw_topic,
        "clickhouse-events-raw",
        &kafka_cfg.kafka_connection,
    );

    let preprocessed_kafka_settings = build_kafka_settings(
        &kafka_cfg.kafka_internal_addr,
        &kafka_cfg.kafka_preprocessed_topic,
        "clickhouse-events-preprocessed",
        &kafka_cfg.kafka_connection,
    );

    let cluster_name = &clickhouse_cfg.cluster_name;
    let cluster_clause = format!(" ON CLUSTER '{cluster_name}'");
    let raw_events_engine = "ReplicatedMergeTree('/clickhouse/tables/{cluster}/{database}/raw_events', '{replica}')";
    let preprocessed_events_engine = "ReplicatedReplacingMergeTree('/clickhouse/tables/{cluster}/{database}/preprocessed_events', '{replica}', timestamp)";

    format!(
        r#"
        -- Reset
        DROP TABLE IF EXISTS meteroid.raw_kafka_events_mv{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events_mv{cluster_clause};
        DROP TABLE IF EXISTS meteroid.raw_kafka_events{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events{cluster_clause};
        DROP TABLE IF EXISTS meteroid.raw_events{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_events{cluster_clause};

        -- Recreate main tables with appropriate engines
        CREATE TABLE IF NOT EXISTS meteroid.raw_events{cluster_clause} (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = {raw_events_engine}
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, timestamp, code, customer_id);

        CREATE TABLE IF NOT EXISTS meteroid.preprocessed_events{cluster_clause} (
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
        ) ENGINE = {preprocessed_events_engine}
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, code, billable_metric_id, customer_id, toDate(timestamp), timestamp, id);

        -- Create Kafka tables with SASL authentication
        CREATE TABLE IF NOT EXISTS meteroid.raw_kafka_events{cluster_clause} (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = Kafka()
          SETTINGS
              {raw_kafka_settings};

        CREATE TABLE IF NOT EXISTS meteroid.preprocessed_kafka_events{cluster_clause} (
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
              {preprocessed_kafka_settings};

        -- Create materialized views
        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.raw_kafka_events_mv{cluster_clause} TO meteroid.raw_events AS
            SELECT * FROM meteroid.raw_kafka_events;

        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.preprocessed_kafka_events_mv{cluster_clause} TO meteroid.preprocessed_events AS
            SELECT * FROM meteroid.preprocessed_kafka_events;
    "#,
        cluster_clause = cluster_clause,
        raw_events_engine = raw_events_engine,
        preprocessed_events_engine = preprocessed_events_engine,
        raw_kafka_settings = raw_kafka_settings,
        preprocessed_kafka_settings = preprocessed_kafka_settings
    )
}
