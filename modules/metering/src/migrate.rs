/// based on klickhouse implementation, except uses native client and ReplicatedMergeTree for cluster option
use clickhouse::Client;
use refinery_core::Migration;
use refinery_core::traits::r#async::{AsyncMigrate, AsyncQuery, AsyncTransaction};
use serde::Deserialize;
use std::marker::PhantomData;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::time::sleep;

#[derive(Error, Debug)]
pub enum MigrationError {
    #[error("ClickHouse error: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error("Migration execution failed: {0}")]
    Execution(String),
}

/// copied from refinery_core
#[allow(dead_code)]
enum State {
    Applied,
    Unapplied,
}

/// copied from refinery_core
#[allow(dead_code)]
enum TypeInner {
    Versioned,
    Unversioned,
}

/// copied from refinery_core - matches Migration memory layout
#[allow(dead_code)]
struct MigrationInner {
    state: State,
    name: String,
    checksum: u64,
    version: i32,
    prefix: TypeInner,
    sql: Option<String>,
    applied_on: Option<time::OffsetDateTime>,
}

impl MigrationInner {
    fn applied(
        version: i32,
        name: String,
        applied_on: time::OffsetDateTime,
        checksum: u64,
    ) -> MigrationInner {
        MigrationInner {
            state: State::Applied,
            name,
            checksum,
            version,
            prefix: TypeInner::Versioned,
            sql: None,
            applied_on: Some(applied_on),
        }
    }
}

impl From<MigrationInner> for Migration {
    fn from(inner: MigrationInner) -> Self {
        assert_eq!(size_of::<Migration>(), size_of::<MigrationInner>());
        unsafe { std::mem::transmute(inner) }
    }
}

/// Intermediate struct for deserializing migrations from ClickHouse
#[derive(Debug, Deserialize, clickhouse::Row)]
struct AppliedMigration {
    version: i32,
    name: String,
    applied_on: String,
    checksum: String,
}

impl AppliedMigration {
    #[allow(clippy::wrong_self_convention)]
    fn to_migration(self) -> Result<Migration, MigrationError> {
        let applied_on = time::OffsetDateTime::parse(
            &self.applied_on,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|e| {
            MigrationError::Execution(format!("Failed to parse applied_on time: {}", e))
        })?;

        let checksum = self
            .checksum
            .parse::<u64>()
            .map_err(|e| MigrationError::Execution(format!("Failed to parse checksum: {}", e)))?;

        Ok(MigrationInner::applied(self.version, self.name, applied_on, checksum).into())
    }
}

/// Wrapper for native ClickHouse client to implement refinery traits
pub struct ClickhouseMigration {
    client: Client,
}

impl ClickhouseMigration {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl AsyncTransaction for ClickhouseMigration {
    type Error = MigrationError;

    async fn execute(&mut self, queries: &[&str]) -> Result<usize, Self::Error> {
        // Acquire lock for migrations
        let lock = ClickhouseLock::new(self.client.clone(), "refinery_exec");
        let start = Instant::now();
        let handle = loop {
            if let Some(handle) = lock.try_lock().await? {
                break handle;
            } else {
                sleep(Duration::from_millis(250)).await;
                if start.elapsed() > Duration::from_secs(60) {
                    lock.reset().await?;
                }
            }
        };

        // Execute queries - split by semicolon for multi-statement support
        for query in queries {
            if query.is_empty() {
                continue;
            }
            // Split by semicolon and execute each statement individually
            for statement in query.split(';') {
                let statement = statement.trim();
                if statement.is_empty() {
                    continue;
                }
                self.client.query(statement).execute().await.map_err(|e| {
                    MigrationError::Execution(format!("Failed to execute query: {}", e))
                })?;
            }
        }

        // Unlock
        handle.unlock().await?;
        Ok(queries.len())
    }
}

#[async_trait::async_trait]
impl AsyncQuery<Vec<Migration>> for ClickhouseMigration {
    async fn query(
        &mut self,
        query: &str,
    ) -> Result<Vec<Migration>, <Self as AsyncTransaction>::Error> {
        // Execute query and decode result into Vec<AppliedMigration>, then convert
        let applied: Vec<AppliedMigration> =
            self.client.query(query).fetch_all().await.map_err(|e| {
                MigrationError::Execution(format!("Failed to query migrations: {}", e))
            })?;

        applied.into_iter().map(|m| m.to_migration()).collect()
    }
}

impl AsyncMigrate for ClickhouseMigration {
    fn assert_migrations_table_query(migration_table_name: &str) -> String {
        format!(
            "CREATE TABLE IF NOT EXISTS {migration_table_name}(
            version INT,
            name String,
            applied_on String,
            checksum String) Engine=MergeTree() ORDER BY version;"
        )
    }
}

pub trait ClusterName: Send + Sync {
    fn cluster_name() -> String;
}

/// Cluster-aware migration wrapper using ReplicatedMergeTree
pub struct ClusterMigration<T: ClusterName> {
    client: Client,
    _t: PhantomData<T>,
}

impl<T: ClusterName> ClusterMigration<T> {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            _t: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl<T: ClusterName> AsyncTransaction for ClusterMigration<T> {
    type Error = MigrationError;

    async fn execute(&mut self, queries: &[&str]) -> Result<usize, Self::Error> {
        // Acquire lock for cluster migrations
        let lock = ClickhouseLock::new(self.client.clone(), "refinery_exec")
            .with_cluster(T::cluster_name());
        let start = Instant::now();
        let handle = loop {
            if let Some(handle) = lock.try_lock().await? {
                break handle;
            } else {
                sleep(Duration::from_millis(250)).await;
                if start.elapsed() > Duration::from_secs(60) {
                    lock.reset().await?;
                }
            }
        };

        // Execute queries - split by semicolon for multi-statement support
        for query in queries {
            if query.is_empty() {
                continue;
            }
            // Split by semicolon and execute each statement individually
            for statement in query.split(';') {
                let statement = statement.trim();
                if statement.is_empty() {
                    continue;
                }
                self.client.query(statement).execute().await.map_err(|e| {
                    MigrationError::Execution(format!("Failed to execute query: {}", e))
                })?;
            }
        }

        // Unlock
        handle.unlock().await?;
        Ok(queries.len())
    }
}

#[async_trait::async_trait]
impl<T: ClusterName> AsyncQuery<Vec<Migration>> for ClusterMigration<T> {
    async fn query(
        &mut self,
        query: &str,
    ) -> Result<Vec<Migration>, <Self as AsyncTransaction>::Error> {
        // Execute query and decode result into Vec<AppliedMigration>, then convert
        let applied: Vec<AppliedMigration> =
            self.client.query(query).fetch_all().await.map_err(|e| {
                MigrationError::Execution(format!("Failed to query migrations: {}", e))
            })?;

        applied.into_iter().map(|m| m.to_migration()).collect()
    }
}

impl<T: ClusterName> AsyncMigrate for ClusterMigration<T> {
    fn assert_migrations_table_query(migration_table_name: &str) -> String {
        // Use ReplicatedMergeTree for proper replication in cluster mode
        format!(
            r"CREATE TABLE IF NOT EXISTS {migration_table_name} ON CLUSTER '{cluster}'(
                version INT,
                name String,
                applied_on String,
                checksum String
            ) Engine=ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/{migration_table_name}', '{{replica}}')
            ORDER BY version;",
            cluster = T::cluster_name(),
            migration_table_name = migration_table_name
        )
    }
}

/// Simple lock implementation using ClickHouse tables
#[derive(Clone)]
struct ClickhouseLock {
    name: String,
    cluster_str: String,
    client: Client,
}

struct ClickhouseLockHandle {
    lock: Option<ClickhouseLock>,
}

impl ClickhouseLock {
    fn new(client: Client, name: impl AsRef<str>) -> Self {
        Self {
            name: name.as_ref().to_string(),
            client,
            cluster_str: String::new(),
        }
    }

    fn with_cluster(mut self, cluster: impl AsRef<str>) -> Self {
        self.cluster_str = format!(" ON CLUSTER '{}'", cluster.as_ref());
        self
    }

    async fn try_lock(&self) -> Result<Option<ClickhouseLockHandle>, MigrationError> {
        let query = format!(
            "CREATE TABLE _lock_{}{} (i Int64) ENGINE=Null",
            self.name, self.cluster_str
        );

        match self.client.query(&query).execute().await {
            Ok(_) => Ok(Some(ClickhouseLockHandle {
                lock: Some(self.clone()),
            })),
            Err(e) => {
                let error = e.to_string();
                if error.contains("already exists") {
                    Ok(None)
                } else {
                    Err(MigrationError::Execution(format!(
                        "Failed to acquire lock: {}",
                        error
                    )))
                }
            }
        }
    }

    async fn reset(&self) -> Result<(), MigrationError> {
        let query = format!(
            "DROP TABLE IF EXISTS _lock_{}{} SYNC",
            self.name, self.cluster_str
        );
        self.client
            .query(&query)
            .execute()
            .await
            .map_err(|e| MigrationError::Execution(format!("Failed to reset lock: {}", e)))?;
        Ok(())
    }
}

impl ClickhouseLockHandle {
    async fn unlock(mut self) -> Result<(), MigrationError> {
        if let Some(lock) = self.lock.take() {
            lock.reset().await?;
        }
        Ok(())
    }
}

impl Drop for ClickhouseLockHandle {
    fn drop(&mut self) {
        if let Some(lock) = self.lock.take() {
            tokio::spawn(async move {
                if let Err(e) = lock.reset().await {
                    log::error!("Failed to reset lock {}: {:?}", lock.name, e);
                }
            });
        }
    }
}
