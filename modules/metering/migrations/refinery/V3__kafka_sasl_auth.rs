use crate::migrations::get_kafka_config;
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
            settings.push(format!("kafka_sasl_mechanisms = '{}'", sasl_mechanism));
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
    let cfg = get_kafka_config();

    let raw_kafka_settings = build_kafka_settings(
        &cfg.kafka_internal_addr,
        &cfg.kafka_raw_topic,
        "clickhouse-events-raw",
        &cfg.kafka_connection,
    );

    let preprocessed_kafka_settings = build_kafka_settings(
        &cfg.kafka_internal_addr,
        &cfg.kafka_preprocessed_topic,
        "clickhouse-events-preprocessed",
        &cfg.kafka_connection,
    );

    format!(
        r#"
        -- Drop materialized views first
        DROP TABLE IF EXISTS meteroid.raw_kafka_events_mv;
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events_mv;

        -- Drop old Kafka tables
        DROP TABLE IF EXISTS meteroid.raw_kafka_events;
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events;

        -- Recreate raw_kafka_events table with SASL authentication
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
              {};

        -- Recreate preprocessed_kafka_events table with SASL authentication
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
              {};

        -- Recreate materialized views
        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.raw_kafka_events_mv TO meteroid.raw_events AS
            SELECT * FROM meteroid.raw_kafka_events;

        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.preprocessed_kafka_events_mv TO meteroid.preprocessed_events AS
            SELECT * FROM meteroid.preprocessed_kafka_events;
    "#,
        raw_kafka_settings, preprocessed_kafka_settings
    )
}
