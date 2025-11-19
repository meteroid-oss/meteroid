use crate::migrations::{get_clickhouse_config, get_kafka_config};
use kafka::config::KafkaConnectionConfig;

fn build_kafka_settings(
    broker_list: &str,
    topic: &str,
    group_name: &str,
    kafka_connection: &KafkaConnectionConfig,
) -> String {
    let mut settings = vec![
        format!("kafka_broker_list = '{}'", broker_list),
        format!("kafka_topic_list = '{}'", topic),
        format!("kafka_group_name = '{}'", group_name),
        "kafka_format = 'JSONEachRow'".to_string(),
    ];

    // Add SASL authentication settings if configured
    if let Some(ref sasl_username) = kafka_connection.sasl_username {
        if !sasl_username.is_empty() {
            settings.push(format!("kafka_sasl_username = '{}'", sasl_username));
        }
    }

    if let Some(ref sasl_password) = kafka_connection.sasl_password {
        if !sasl_password.is_empty() {
            settings.push(format!("kafka_sasl_password = '{}'", sasl_password));
        }
    }

    if let Some(ref sasl_mechanism) = kafka_connection.sasl_mechanism {
        if !sasl_mechanism.is_empty() {
            settings.push(format!("kafka_sasl_mechanism = '{}'", sasl_mechanism));
        }
    }

    if let Some(ref security_protocol) = kafka_connection.security_protocol {
        if !security_protocol.is_empty() {
            settings.push(format!("kafka_security_protocol = '{}'", security_protocol));
        }
    }

    settings.join(",\n              ")
}

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

    // Conditionally use ON CLUSTER based on configuration
    let cluster_clause = if let Some(ref cluster_name) = clickhouse_cfg.cluster_name {
        format!(" ON CLUSTER '{}'", cluster_name)
    } else {
        String::new()
    };

    // Use replicated engines only if we have a cluster
    let (raw_events_engine, preprocessed_events_engine) = if clickhouse_cfg.cluster_name.is_some() {
        (
            "ReplicatedMergeTree('/clickhouse/tables/{cluster}/{database}/raw_events', '{replica}')".to_string(),
            "ReplicatedReplacingMergeTree('/clickhouse/tables/{cluster}/{database}/preprocessed_events', '{replica}', timestamp)".to_string(),
        )
    } else {
        (
            "MergeTree()".to_string(),
            "ReplacingMergeTree(timestamp)".to_string(),
        )
    };

    format!(
        r#"
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
