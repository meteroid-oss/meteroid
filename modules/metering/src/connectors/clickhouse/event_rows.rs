//! The stored copies of event keys, read by `LookupEvent` and by the consumer before it applies
//! amends and voids (ADR-0012 §4).
use crate::config::EventTablesConfig;
use chrono::{DateTime, Utc};
use clickhouse::query::Query;
use clickhouse::sql::Identifier;
use clickhouse::{Client, Row};
use metric_core::sql::EVENT_VOIDS_TABLE;
use serde::Deserialize;
use std::collections::BTreeMap;
use uuid::Uuid;

/// `(tenant_id, customer_id, id)` as SQL literals.
pub type LookupKey = (String, String, String);

/// A stored copy that is not a tombstone; `voided` once a tombstone cancelled it.
#[derive(Debug, Clone, Deserialize, Row, PartialEq)]
pub struct StoredRow {
    #[serde(with = "clickhouse::serde::uuid")]
    pub tenant_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    pub customer_id: Uuid,
    pub id: String,
    pub code: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub timestamp: DateTime<Utc>,
    pub properties: BTreeMap<String, String>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub ingested_at: DateTime<Utc>,
    pub voided: u8,
}

/// Reads every replica, skipping unreachable ones: quorum inserts put each acknowledged row on a
/// majority, so the reachable replicas still see it with a minority down.
pub fn across_replicas(query: Query) -> Query {
    query
        .with_setting("skip_unavailable_shards", "1")
        .with_setting("connect_timeout_with_failover_ms", "500")
        .with_setting("connections_with_failover_max_tries", "1")
        .with_setting("handshake_timeout_ms", "2000")
        .with_setting("max_execution_time", "5")
        // The key list is bound three times; a chunk stays far below this.
        .with_setting("max_query_size", (4 * 1024 * 1024).to_string())
}

/// The keys' timestamps come from the ids of the 35-day window, so the raw read prunes to their
/// partitions. Sorted by key and `ingested_at`, one row per stored copy.
pub async fn lookup(
    client: &Client,
    cluster: &str,
    tables: &EventTablesConfig,
    keys: &[LookupKey],
) -> Result<Vec<StoredRow>, clickhouse::error::Error> {
    let key = "(tenant_id, customer_id, id) IN ?";
    let sql = format!(
        "SELECT tenant_id, customer_id, id, code, timestamp, properties, ingested_at, \
         toUInt8((tenant_id, customer_id, id, ingested_at) GLOBAL IN (SELECT tenant_id, \
         customer_id, id, ingested_at FROM clusterAllReplicas(?, currentDatabase(), ?) WHERE {key})) \
         AS voided FROM clusterAllReplicas(?, currentDatabase(), ?) WHERE {key} AND is_tombstone = 0 \
         AND timestamp GLOBAL IN (SELECT timestamp FROM clusterAllReplicas(?, currentDatabase(), ?) \
         WHERE {key} AND ingested_at >= now64(9) - INTERVAL 35 DAY)"
    );
    let mut query = client.query(&sql);
    for table in [
        EVENT_VOIDS_TABLE,
        tables.raw_events_local_table.as_str(),
        tables.event_ids_local_table.as_str(),
    ] {
        query = query.bind(cluster).bind(Identifier(table)).bind(keys);
    }
    let mut rows = across_replicas(query).fetch_all::<StoredRow>().await?;
    let order = |row: &StoredRow| {
        (
            row.tenant_id,
            row.customer_id,
            row.id.clone(),
            row.ingested_at,
        )
    };
    rows.sort_by_key(order);
    rows.dedup_by_key(|row| order(row));
    Ok(rows)
}

/// Fails when the user cannot read a table the change lookup reads, rather than on the first change.
pub async fn check_access(
    client: &Client,
    cluster: &str,
    tables: &EventTablesConfig,
) -> Result<(), String> {
    for table in [
        EVENT_VOIDS_TABLE,
        tables.raw_events_local_table.as_str(),
        tables.event_ids_local_table.as_str(),
    ] {
        let query = client
            .query("SELECT * FROM clusterAllReplicas(?, currentDatabase(), ?) LIMIT 0")
            .bind(cluster)
            .bind(Identifier(table));
        across_replicas(query)
            .execute()
            .await
            .map_err(|e| format!("cannot read {table} through clusterAllReplicas: {e}"))?;
    }
    Ok(())
}
