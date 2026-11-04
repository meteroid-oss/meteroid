//! ClickHouse cluster migration runner (refinery-core backend, native client).
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

// Replicate refinery_core's private `Migration` memory layout so an applied migration can be reconstructed via transmute (no public constructor).
#[allow(dead_code)]
enum State {
    Applied,
    Unapplied,
}

#[allow(dead_code)]
enum TypeInner {
    Versioned,
    Unversioned,
}

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

pub trait ClusterName: Send + Sync {
    fn cluster_name() -> String;
}

/// Split on top-level `;` only, skipping string literals, quoted identifiers and comments (which are dropped).
fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' => {
                // Copy the quoted span verbatim, honoring `\x` and doubled-quote escapes.
                current.push(c);
                while let Some(n) = chars.next() {
                    current.push(n);
                    if n == '\\' {
                        if let Some(e) = chars.next() {
                            current.push(e);
                        }
                    } else if n == c {
                        if chars.peek() == Some(&c) {
                            current.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                // Line comment: drop through end of line (the newline is preserved).
                chars.next();
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                // Block comment: drop through `*/`, leaving a space as a token separator.
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                current.push(' ');
            }
            ';' => {
                let stmt = current.trim();
                if !stmt.is_empty() {
                    statements.push(stmt.to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }

    let tail = current.trim();
    if !tail.is_empty() {
        statements.push(tail.to_string());
    }

    statements
}

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
impl<TR: ClusterName> AsyncTransaction for ClusterMigration<TR> {
    type Error = MigrationError;

    async fn execute<'a, T: Iterator<Item = &'a str> + Send>(
        &mut self,
        queries: T,
    ) -> Result<usize, Self::Error> {
        let handle = lock(self.client.clone(), "refinery_exec", &TR::cluster_name()).await?;

        let mut count = 0;
        for query in queries {
            if query.is_empty() {
                continue;
            }
            count += 1;
            for statement in split_sql_statements(query) {
                self.client.query(&statement).execute().await.map_err(|e| {
                    MigrationError::Execution(format!("Failed to execute query: {}", e))
                })?;
            }
        }

        handle.unlock().await?;
        Ok(count)
    }
}

#[async_trait::async_trait]
impl<T: ClusterName> AsyncQuery<Vec<Migration>> for ClusterMigration<T> {
    async fn query(
        &mut self,
        query: &str,
    ) -> Result<Vec<Migration>, <Self as AsyncTransaction>::Error> {
        let applied: Vec<AppliedMigration> =
            self.client.query(query).fetch_all().await.map_err(|e| {
                MigrationError::Execution(format!("Failed to query migrations: {}", e))
            })?;

        applied.into_iter().map(|m| m.to_migration()).collect()
    }
}

impl<T: ClusterName> AsyncMigrate for ClusterMigration<T> {
    fn assert_migrations_table_query(migration_table_name: &str) -> String {
        let local_table = format!("{migration_table_name}_local");
        let cluster = T::cluster_name();
        format!(
            r"CREATE TABLE IF NOT EXISTS {local_table} ON CLUSTER '{cluster}'(
                version INT,
                name String,
                applied_on String,
                checksum String
            ) Engine=ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/{local_table}', '{{replica}}')
            ORDER BY version;
            CREATE TABLE IF NOT EXISTS {migration_table_name} ON CLUSTER '{cluster}'(
                version INT,
                name String,
                applied_on String,
                checksum String
            ) ENGINE = Distributed('{cluster}', currentDatabase(), '{local_table}', rand());",
            cluster = cluster,
            local_table = local_table,
            migration_table_name = migration_table_name
        )
    }
}

/// Waits for the cluster-wide lock `name`, taken over after 60s from a holder that died.
pub async fn lock(
    client: Client,
    name: &str,
    cluster: &str,
) -> Result<ClickhouseLockHandle, MigrationError> {
    let lock = ClickhouseLock::new(client, name).with_cluster(cluster);
    let start = Instant::now();
    loop {
        if let Some(handle) = lock.try_lock().await? {
            return Ok(handle);
        }
        sleep(Duration::from_millis(250)).await;
        if start.elapsed() > Duration::from_secs(60) {
            lock.reset().await?;
        }
    }
}

#[derive(Clone)]
struct ClickhouseLock {
    name: String,
    cluster_str: String,
    client: Client,
}

pub struct ClickhouseLockHandle {
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
    pub async fn unlock(mut self) -> Result<(), MigrationError> {
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

#[cfg(test)]
mod tests {
    use super::split_sql_statements;
    use rstest::rstest;

    #[rstest]
    #[case(
        "CREATE TABLE a (x Int64); CREATE TABLE b (y Int64);",
        &["CREATE TABLE a (x Int64)", "CREATE TABLE b (y Int64)"]
    )]
    #[case(
        "INSERT INTO t VALUES ('a;b', '--not-a-comment'); SELECT 1;",
        &["INSERT INTO t VALUES ('a;b', '--not-a-comment')", "SELECT 1"]
    )]
    #[case(
        "SELECT 'it''s; fine', 'esc\\'; still'; SELECT 2;",
        &["SELECT 'it''s; fine', 'esc\\'; still'", "SELECT 2"]
    )]
    #[case(
        "CREATE TABLE a (x Int64) /* drop; this */ ENGINE = Null;",
        &["CREATE TABLE a (x Int64)   ENGINE = Null"]
    )]
    #[case("SELECT 1;\n-- trailing comment only\n", &["SELECT 1"])]
    fn split_sql_exact(#[case] sql: &str, #[case] expected: &[&str]) {
        assert_eq!(split_sql_statements(sql), expected);
    }

    #[test]
    fn ignores_semicolon_inside_line_comment() {
        let sql = "CREATE TABLE a (\n  x Int64,\n  -- re-derives a trailing window; latest wins\n  y Int64\n);";
        let out = split_sql_statements(sql);
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("CREATE TABLE a ("));
        assert!(out[0].ends_with("y Int64\n)"));
        assert!(!out[0].contains("trailing window"));
    }

    #[test]
    fn protects_dashes_in_zookeeper_path_string() {
        let sql = "CREATE TABLE t ON CLUSTER 'c' (x Int64) ENGINE = ReplicatedMergeTree('/clickhouse/tables/{cluster}/{database}/t', '{replica}');";
        let out = split_sql_statements(sql);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("/clickhouse/tables/{cluster}/{database}/t"));
    }
}
