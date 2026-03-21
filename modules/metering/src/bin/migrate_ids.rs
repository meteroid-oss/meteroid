/// Migrates raw_events_local (base62 string IDs) → raw_events_local_v2 (UUID).
///
/// Run after the V7 schema migration. The old table is left intact; drop it
/// manually once you have verified the migrated data.
///
/// Env vars: same ClickHouse vars as the metering server
///   CLICKHOUSE_HTTP_ADDRESS, CLICKHOUSE_USERNAME, CLICKHOUSE_PASSWORD,
///   CLICKHOUSE_DATABASE, CLICKHOUSE_CLUSTER_NAME
///
/// Optional:
///   BATCH_SIZE  (default: 10_000)
use clickhouse::Row;
use common_domain::ids::{CustomerId, TenantId};
use envconfig::Envconfig;
use metering::config::ClickhouseConfig;
use metering::ingest::domain::RawEventRow;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, Serialize, Deserialize, Row)]
struct OldRawEventRow {
    pub id: String,
    pub code: String,
    pub customer_id: String,
    pub tenant_id: String,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
    pub ingested_at: chrono::DateTime<chrono::Utc>,
    pub properties: Vec<(String, String)>,
}

impl TryFrom<OldRawEventRow> for RawEventRow {
    type Error = String;

    fn try_from(old: OldRawEventRow) -> Result<Self, Self::Error> {
        let customer_id = CustomerId::from_str(&old.customer_id)
            .map_err(|e| format!("invalid customer_id '{}': {}", old.customer_id, e))?;
        let tenant_id = TenantId::from_str(&old.tenant_id)
            .map_err(|e| format!("invalid tenant_id '{}': {}", old.tenant_id, e))?;

        Ok(RawEventRow {
            id: old.id,
            code: old.code,
            customer_id: *customer_id,
            tenant_id: *tenant_id,
            timestamp: old.timestamp,
            ingested_at: old.ingested_at,
            properties: old.properties.into_iter().collect(),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match dotenvy::dotenv() {
        Err(e) if e.not_found() => {}
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }

    let batch_size: usize = std::env::var("BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let cfg = ClickhouseConfig::init_from_env()?;

    let client = clickhouse::Client::default()
        .with_url(&cfg.http_address)
        .with_user(&cfg.username)
        .with_password(&cfg.password)
        .with_database(&cfg.database);

    let total: u64 = client
        .query("SELECT count() FROM meteroid.raw_events_local")
        .fetch_one()
        .await?;

    println!("Total rows to migrate: {total}");

    let mut migrated: u64 = 0;
    let mut errors: u64 = 0;
    let mut offset: u64 = 0;

    loop {
        let batch: Vec<OldRawEventRow> = client
            .query(
                "SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties \
                 FROM meteroid.raw_events_local \
                 ORDER BY (tenant_id, timestamp) \
                 LIMIT ? OFFSET ?",
            )
            .bind(batch_size as u64)
            .bind(offset)
            .fetch_all()
            .await?;

        if batch.is_empty() {
            break;
        }

        let fetched = batch.len() as u64;

        let mut inserter = client.inserter::<RawEventRow>("meteroid.raw_events_local_v2");

        for old_row in batch {
            match RawEventRow::try_from(old_row) {
                Ok(new_row) => {
                    inserter.write(&new_row).await?;
                    migrated += 1;
                }
                Err(e) => {
                    eprintln!("Skipping row: {e}");
                    errors += 1;
                }
            }
        }

        inserter.end().await?;

        offset += fetched;
        println!("Progress: {offset}/{total} (errors: {errors})");

        if fetched < batch_size as u64 {
            break;
        }
    }

    println!("Done. Migrated: {migrated}, errors: {errors}");

    Ok(())
}
