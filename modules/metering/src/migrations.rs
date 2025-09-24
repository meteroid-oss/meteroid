use crate::config::{ClickhouseConfig, KafkaConfig};
use error_stack::ResultExt;
use klickhouse::{Client, ClientOptions};
use thiserror::Error;

static KAFKA_CONFIG: std::sync::OnceLock<KafkaConfig> = std::sync::OnceLock::new();
static CLICKHOUSE_CONFIG: std::sync::OnceLock<ClickhouseConfig> = std::sync::OnceLock::new();

refinery::embed_migrations!("migrations/refinery");

pub async fn run(
    clickhouse_config: &ClickhouseConfig,
    kafka_config: &KafkaConfig,
) -> error_stack::Result<(), MigrationsError> {
    set_kafka_config(kafka_config);
    set_clickhouse_config(clickhouse_config);

    let mut client = Client::connect(
        &clickhouse_config.tcp_address,
        ClientOptions {
            username: clickhouse_config.username.clone(),
            password: clickhouse_config.password.clone(),
            default_database: clickhouse_config.database.clone(),
            tcp_nodelay: true,
        },
    )
    .await
    .change_context(MigrationsError::Execution)?;

    let report = migrations::runner()
        .run_async(&mut client)
        .await
        .change_context(MigrationsError::Execution)?;

    for migration in report.applied_migrations() {
        log::info!("Migration {} has been applied", migration);
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
