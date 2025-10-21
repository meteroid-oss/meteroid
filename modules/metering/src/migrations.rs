use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::migrate::{ClusterMigration, ClusterName, sync_checksums};
use clickhouse::Client;
use error_stack::{Report, ResultExt};
use kafka::config::KafkaConnectionConfig;
use thiserror::Error;

static KAFKA_CONFIG: std::sync::OnceLock<KafkaConfig> = std::sync::OnceLock::new();
static CLICKHOUSE_CONFIG: std::sync::OnceLock<ClickhouseConfig> = std::sync::OnceLock::new();

pub fn build_kafka_settings(
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

    if let Some(ref v) = kafka_connection.sasl_username
        && !v.is_empty()
    {
        settings.push(format!("kafka_sasl_username = '{}'", v));
    }
    if let Some(ref v) = kafka_connection.sasl_password
        && !v.is_empty()
    {
        settings.push(format!("kafka_sasl_password = '{}'", v));
    }
    if let Some(ref v) = kafka_connection.sasl_mechanism
        && !v.is_empty()
    {
        settings.push(format!("kafka_sasl_mechanism = '{}'", v));
    }
    if let Some(ref v) = kafka_connection.security_protocol
        && !v.is_empty()
    {
        settings.push(format!("kafka_security_protocol = '{}'", v));
    }

    settings.join(",\n              ")
}

refinery::embed_migrations!("migrations/refinery");

pub async fn run(
    clickhouse_config: &ClickhouseConfig,
    kafka_config: &KafkaConfig,
) -> Result<(), Report<MigrationsError>> {
    set_kafka_config(kafka_config);
    set_clickhouse_config(clickhouse_config);

    // Create native ClickHouse HTTP client
    let client = Client::default()
        .with_url(&clickhouse_config.http_address)
        .with_user(&clickhouse_config.username)
        .with_password(&clickhouse_config.password)
        .with_database(&clickhouse_config.database);

    let mut runner = &mut migrations::runner();
    runner = runner.set_migration_table_name("refinery_schema_history");

    // temporary: syncs checksums that drifted due to dynamic SQL in V1-V5.
    // becomes a no-op once V6 has run in all environments.
    sync_checksums(&client, "refinery_schema_history", runner.get_migrations())
        .await
        .change_context(MigrationsError::Execution)?;

    struct MeteroidCluster;
    impl ClusterName for MeteroidCluster {
        fn cluster_name() -> String {
            get_clickhouse_config().cluster_name.clone()
        }
    }

    let mut cluster_client = ClusterMigration::<MeteroidCluster>::new(client.clone());

    let report = runner
        .run_async(&mut cluster_client)
        .await
        .change_context(MigrationsError::Execution)?;

    for migration in report.applied_migrations() {
        log::info!("Applied migration: {migration}");
    }

    Ok(())
}

fn set_kafka_config(config: &KafkaConfig) {
    KAFKA_CONFIG
        .set(config.clone())
        .unwrap_or_else(|_| log::warn!("KAFKA_CONFIG already set, ignoring new value"));
}

fn set_clickhouse_config(config: &ClickhouseConfig) {
    CLICKHOUSE_CONFIG
        .set(config.clone())
        .unwrap_or_else(|_| log::warn!("CLICKHOUSE_CONFIG already set, ignoring new value"));
}

pub fn get_kafka_config() -> &'static KafkaConfig {
    KAFKA_CONFIG
        .get()
        .expect("KafkaConfig should be initialized before use")
}

pub fn get_clickhouse_config() -> &'static ClickhouseConfig {
    CLICKHOUSE_CONFIG
        .get()
        .expect("ClickhouseConfig should be initialized before use")
}

#[derive(Error, Debug)]
pub enum MigrationsError {
    #[error("failed to run migrations")]
    Execution,
}
