use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::migrate::{ClickhouseMigration, ClusterMigration, ClusterName};
use clickhouse::Client;
use error_stack::{Report, ResultExt};
use thiserror::Error;

static KAFKA_CONFIG: std::sync::OnceLock<KafkaConfig> = std::sync::OnceLock::new();
static CLICKHOUSE_CONFIG: std::sync::OnceLock<ClickhouseConfig> = std::sync::OnceLock::new();

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
    runner = runner.set_migration_table_name("refinery_schema_history_v2");

    if clickhouse_config.cluster_name.is_some() {
        struct MeteroidCluster;
        impl ClusterName for MeteroidCluster {
            fn cluster_name() -> String {
                get_clickhouse_config()
                    .cluster_name
                    .as_ref()
                    .expect("cluster_name should be set for cluster migrations")
                    .clone()
            }
        }

        let mut cluster_client = ClusterMigration::<MeteroidCluster>::new(client);

        let report = runner
            .run_async(&mut cluster_client)
            .await
            .change_context(MigrationsError::Execution)?;

        for migration in report.applied_migrations() {
            log::info!("Migration {migration} has been applied");
        }

        return Ok(());
    }

    // Single-node mode using regular MergeTree
    let mut single_client = ClickhouseMigration::new(client);

    let report = runner
        .run_async(&mut single_client)
        .await
        .change_context(MigrationsError::Execution)?;

    for migration in report.applied_migrations() {
        log::info!("Migration {migration} has been applied");
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
