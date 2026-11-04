//! ClickHouse history left by the releases before 1.0.0-rc4 (0.19.x, 1.0.0-rc1 to rc3), which
//! ends at V4.

use super::MigrationsError;
use crate::ingest::domain::RawEventRow;
use clickhouse::{Client, Row};
use common_domain::ids::{CustomerId, TenantId};
use error_stack::{Report, ResultExt};
use refinery::{Runner, SchemaVersion};
use serde::Deserialize;
use std::str::FromStr;

/// Their SQL is generated, embeds the Kafka settings, and changed in 1.0.0-rc4.
const GENERATED_BEFORE_RC4: std::ops::RangeInclusive<SchemaVersion> = 1..=4;
pub(super) const IDS_TO_UUID: SchemaVersion = 7;
const DROP_BASE62_EVENTS: SchemaVersion = 8;
const UNCONVERTED_TABLE: &str = "raw_events_unconverted";
const PROGRESS_EVERY: u64 = 1_000_000;

#[derive(Row, Deserialize)]
struct AppliedChecksum {
    version: SchemaVersion,
    checksum: String,
}

#[derive(Row, Deserialize)]
struct Base62EventRow {
    id: String,
    code: String,
    customer_id: String,
    tenant_id: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    ingested_at: chrono::DateTime<chrono::Utc>,
    properties: Vec<(String, String)>,
}

impl TryFrom<Base62EventRow> for RawEventRow {
    type Error = String;

    fn try_from(row: Base62EventRow) -> Result<Self, Self::Error> {
        let customer_id = CustomerId::from_str(&row.customer_id)
            .map_err(|e| format!("customer_id `{}`: {e}", row.customer_id))?;
        let tenant_id = TenantId::from_str(&row.tenant_id)
            .map_err(|e| format!("tenant_id `{}`: {e}", row.tenant_id))?;
        Ok(RawEventRow {
            id: row.id,
            code: row.code,
            customer_id: *customer_id,
            tenant_id: *tenant_id,
            timestamp: row.timestamp,
            ingested_at: row.ingested_at,
            properties: row.properties.into_iter().collect(),
        })
    }
}

fn failed(what: &str) -> impl FnOnce() -> String + '_ {
    move || format!("legacy ClickHouse history: {what}")
}

async fn table_exists(client: &Client, name: &str) -> Result<bool, Report<MigrationsError>> {
    client
        .query("SELECT count() FROM system.tables WHERE database = currentDatabase() AND name = ?")
        .bind(name)
        .fetch_one::<u64>()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed("system.tables"))
        .map(|n| n > 0)
}

async fn count(client: &Client, table: &str) -> Result<u64, Report<MigrationsError>> {
    client
        .query(&format!("SELECT count() FROM {table}"))
        .fetch_one::<u64>()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed(table))
}

async fn is_applied(
    client: &Client,
    history: &str,
    version: SchemaVersion,
) -> Result<bool, Report<MigrationsError>> {
    client
        .query(&format!("SELECT count() FROM {history} WHERE version = ?"))
        .bind(version)
        .fetch_one::<u64>()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed(history))
        .map(|n| n > 0)
}

/// Stores the current checksums of V1 to V4, so that a history they diverge from runs on. A
/// divergent later migration still stops the run.
pub(super) async fn accept_checksums_before_rc4(
    client: &Client,
    runner: &Runner,
    history: &str,
    cluster: &str,
) -> Result<(), Report<MigrationsError>> {
    if !table_exists(client, history).await? {
        return Ok(());
    }
    let applied: Vec<AppliedChecksum> = client
        .query(&format!("SELECT version, checksum FROM {history}"))
        .fetch_all()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed(history))?;
    let engine: String = client
        .query("SELECT engine FROM system.tables WHERE database = currentDatabase() AND name = ?")
        .bind(history)
        .fetch_one()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed("system.tables"))?;
    // Releases before rc4 kept the history in a plain table; later ones, behind a Distributed one.
    let target = if engine == "Distributed" {
        format!("{history}_local ON CLUSTER '{cluster}'")
    } else {
        history.to_string()
    };

    for migration in runner.get_migrations() {
        let version = migration.version();
        if !GENERATED_BEFORE_RC4.contains(&version) {
            continue;
        }
        let current = migration.checksum().to_string();
        if !applied
            .iter()
            .any(|a| a.version == version && a.checksum != current)
        {
            continue;
        }
        client
            .query(&format!(
                "ALTER TABLE {target} UPDATE checksum = ? WHERE version = ? SETTINGS mutations_sync = 2"
            ))
            .bind(&current)
            .bind(version)
            .execute()
            .await
            .change_context(MigrationsError::Execution)
            .attach_with(failed("checksum update"))?;
        log::info!(
            "ClickHouse migration {migration}: checksum of a release before 1.0.0-rc4 updated"
        );
    }
    Ok(())
}

/// V7 created the UUID-keyed event tables and V8 drops the base62-keyed ones: the release that
/// added them copied the events with a separate command, so this copies them in between. A row
/// whose ids do not parse keeps the old table, renamed, out of V8's reach.
pub(super) async fn copy_base62_events(
    client: &Client,
    history: &str,
    cluster: &str,
) -> Result<(), Report<MigrationsError>> {
    if !is_applied(client, history, IDS_TO_UUID).await?
        || is_applied(client, history, DROP_BASE62_EVENTS).await?
        || !table_exists(client, "raw_events_local").await?
    {
        return Ok(());
    }
    let lock = metric_core::migrate::lock(client.clone(), "legacy_events_copy", cluster)
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed("lock"))?;
    // Another replica may have finished the copy while this one waited.
    if is_applied(client, history, DROP_BASE62_EVENTS).await?
        || !table_exists(client, "raw_events_local").await?
    {
        return lock
            .unlock()
            .await
            .change_context(MigrationsError::Execution);
    }

    // Until V8 runs, ingestion waits for the migrations: these rows are an interrupted copy.
    let leftover = count(client, "raw_events_v2").await?;
    if leftover > 0 {
        log::warn!(
            "Removing the {leftover} usage events of an interrupted copy from raw_events_v2"
        );
        client
            .query(&format!(
                "TRUNCATE TABLE raw_events_local_v2 ON CLUSTER '{cluster}' SYNC"
            ))
            .execute()
            .await
            .change_context(MigrationsError::Execution)
            .attach_with(failed("truncate raw_events_local_v2"))?;
    }

    // An interrupted V8 may have dropped the routing table only; releases before rc4 ran one node.
    let source = if table_exists(client, "raw_events").await? {
        "raw_events"
    } else {
        "raw_events_local"
    };
    let total = count(client, source).await?;
    if total > 0 {
        log::info!("Copying {total} usage events to the UUID-keyed tables");
    }
    let sync_client = client.clone().with_setting("insert_distributed_sync", "1");
    let mut cursor = client
        .query(&format!("SELECT ?fields FROM {source}"))
        .fetch::<Base62EventRow>()
        .change_context(MigrationsError::Execution)
        .attach_with(failed(source))?;
    let mut inserter = sync_client
        .inserter::<RawEventRow>("raw_events_v2")
        .with_max_rows(100_000);
    let (mut copied, mut unconverted) = (0u64, Vec::new());
    let mut unconverted_count = 0u64;

    while let Some(row) = cursor
        .next()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed(source))?
    {
        match RawEventRow::try_from(row) {
            Ok(event) => {
                inserter
                    .write(&event)
                    .await
                    .change_context(MigrationsError::Execution)
                    .attach_with(failed("write raw_events_v2"))?;
                inserter
                    .commit()
                    .await
                    .change_context(MigrationsError::Execution)
                    .attach_with(failed("write raw_events_v2"))?;
                copied += 1;
                if copied.is_multiple_of(PROGRESS_EVERY) {
                    log::info!("Copied {copied} of {total} usage events");
                }
            }
            Err(e) => {
                unconverted_count += 1;
                if unconverted.len() < 5 {
                    unconverted.push(e);
                }
            }
        }
    }
    inserter
        .end()
        .await
        .change_context(MigrationsError::Execution)
        .attach_with(failed("write raw_events_v2"))?;

    if unconverted_count > 0 {
        client
            .query(&format!(
                "RENAME TABLE raw_events_local TO {UNCONVERTED_TABLE} ON CLUSTER '{cluster}'"
            ))
            .execute()
            .await
            .change_context(MigrationsError::Execution)
            .attach_with(failed("rename raw_events_local"))?;
        log::warn!(
            "{unconverted_count} usage events have ids that are not Meteroid ids ({}), and were not \
             copied. All the events of the previous layout are kept in {UNCONVERTED_TABLE}: drop it \
             once reviewed.",
            unconverted.join("; ")
        );
    }
    if total > 0 {
        log::info!("Copied {copied} usage events to the UUID-keyed tables");
    }

    lock.unlock()
        .await
        .change_context(MigrationsError::Execution)
}
