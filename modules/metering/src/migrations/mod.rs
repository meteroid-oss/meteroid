use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::migrate::{ClusterMigration, ClusterName};
use clickhouse::Client;
use error_stack::{Report, ResultExt};
use kafka::config::KafkaConnectionConfig;
use refinery::Target;
use thiserror::Error;

mod legacy;

struct MigrationConfig {
    kafka: KafkaConfig,
    clickhouse: ClickhouseConfig,
}

// Scoped to one `run`, never process-wide: servers sharing a process (the integration tests)
// each migrate their own ClickHouse against their own broker.
tokio::task_local! {
    static CONFIG: MigrationConfig;
}

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
    let config = MigrationConfig {
        kafka: kafka_config.clone(),
        clickhouse: clickhouse_config.clone(),
    };
    CONFIG.scope(config, apply(clickhouse_config)).await
}

const HISTORY_TABLE: &str = "refinery_schema_history";

fn runner(target: Target) -> refinery::Runner {
    let mut runner = migrations::runner();
    runner.set_migration_table_name(HISTORY_TABLE);
    // Some refinery_schema_history rows may be for migrations this runner no longer embeds —
    // analytics migrations that briefly ran here before moving to their own crate and table.
    runner.set_abort_missing(false).set_target(target)
}

async fn apply(clickhouse_config: &ClickhouseConfig) -> Result<(), Report<MigrationsError>> {
    let client = Client::default()
        .with_url(&clickhouse_config.http_address)
        .with_user(&clickhouse_config.username)
        .with_password(&clickhouse_config.password)
        .with_database(&clickhouse_config.database);

    struct MeteroidCluster;
    impl ClusterName for MeteroidCluster {
        fn cluster_name() -> String {
            get_clickhouse_config().cluster_name
        }
    }
    let cluster = MeteroidCluster::cluster_name();
    let mut cluster_client = ClusterMigration::<MeteroidCluster>::new(client.clone());

    legacy::accept_checksums_before_rc4(&client, &runner(Target::Latest), HISTORY_TABLE, &cluster)
        .await?;
    for target in [Target::Version(legacy::IDS_TO_UUID), Target::Latest] {
        let report = runner(target)
            .run_async(&mut cluster_client)
            .await
            .change_context(MigrationsError::Execution)?;
        for migration in report.applied_migrations() {
            log::info!("Applied migration: {migration}");
        }
        if matches!(target, Target::Version(_)) {
            legacy::copy_base62_events(&client, HISTORY_TABLE, &cluster).await?;
        }
    }

    Ok(())
}

pub fn get_kafka_config() -> KafkaConfig {
    CONFIG.with(|config| config.kafka.clone())
}

pub fn get_clickhouse_config() -> ClickhouseConfig {
    CONFIG.with(|config| config.clickhouse.clone())
}

#[derive(Error, Debug)]
pub enum MigrationsError {
    #[error("failed to run migrations")]
    Execution,
}
