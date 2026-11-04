use crate::config::{ClickhouseConfig, KafkaConfig};
use crate::connectors::Connector;
use crate::connectors::errors::ConnectorError;
use crate::domain::{
    CustomerActivity, CustomerFirstEvent, DiscoveredProperty, EventAttribution, EventCodeCount,
    EventPropertiesProfile, PropertyValueCount, QueryMeterParams, QueryRawEventsParams,
    QueryRawEventsResult, TenantEventScan, TenantFirstEvent, Usage, UsageActivityParams,
};
use async_trait::async_trait;
use common_domain::ids::{BaseId, CustomerId, TenantId};
use std::collections::HashMap;

use error_stack::{Report, ResultExt};

use std::sync::Arc;

mod discovery;
pub mod extensions;
pub mod sql;

use crate::connectors::clickhouse::extensions::ConnectorClickhouseExtension;
use crate::connectors::clickhouse::sql::PropertyColumn;

use crate::connectors::json::JsonFieldExtractor;
use crate::migrations;
use clickhouse::Client;
use tokio::io::AsyncBufReadExt;

/// Underlying local (replicated) table behind the `raw_events_v2` Distributed
/// table — mutations must target it. Matches the metering migrations.
const RAW_EVENTS_LOCAL_TABLE: &str = "raw_events_local_v2";

/// Since 25.4, lazy materialization crashes `ORDER BY … LIMIT <= 10` reads of the Distributed
/// events table when served by the local replica. Older servers reject the unknown setting.
const LAZY_MATERIALIZATION: &str = "query_plan_optimize_lazy_materialization";

#[derive(Clone)]
pub struct ClickhouseConnector {
    client: Arc<Client>,
    extensions: Vec<Arc<dyn ConnectorClickhouseExtension + Send + Sync>>,
    events_table: String,
    database: String,
    cluster_name: String,
}

impl ClickhouseConnector {
    pub async fn init(
        clickhouse_config: &ClickhouseConfig,
        kafka_config: &KafkaConfig,
        extensions: Vec<Arc<dyn ConnectorClickhouseExtension + Send + Sync>>,
    ) -> Result<Self, Report<ConnectorError>> {
        migrations::run(clickhouse_config, kafka_config)
            .await
            .change_context(ConnectorError::InitError(
                "Failed to run migrations".to_string(),
            ))?;

        let client = Client::default()
            .with_url(&clickhouse_config.http_address)
            .with_user(&clickhouse_config.username)
            .with_password(&clickhouse_config.password)
            .with_database(&clickhouse_config.database);

        let client = if supports_setting(&client, LAZY_MATERIALIZATION).await? {
            client.with_setting(LAZY_MATERIALIZATION, "0")
        } else {
            client
        };
        let client = Arc::new(client);

        for ext in &extensions {
            ext.init(client.clone()).await?;
        }

        Ok(ClickhouseConnector {
            client,
            extensions,
            events_table: clickhouse_config.raw_events_table.clone(),
            database: clickhouse_config.database.clone(),
            cluster_name: clickhouse_config.cluster_name.clone(),
        })
    }

    fn match_extension(
        &self,
        params: &QueryMeterParams,
    ) -> Option<Arc<dyn ConnectorClickhouseExtension + Send + Sync>> {
        self.extensions
            .iter()
            .find(|ext| params.code.starts_with(&ext.prefix()))
            .cloned()
    }
}

#[async_trait]
impl Connector for ClickhouseConnector {
    #[tracing::instrument(skip_all)]
    async fn query_meter(
        &self,
        params: QueryMeterParams,
    ) -> Result<Vec<Usage>, Report<ConnectorError>> {
        // A bespoke extension builds its own SQL and does NOT apply `params.filters`; running
        // it with filters set would silently over-aggregate (mis-bill). Reject instead.
        let attributed = params
            .attribution
            .as_ref()
            .is_some_and(|attribution| *attribution != EventAttribution::default());
        if attributed && self.match_extension(&params).is_some() {
            return Err(Report::new(ConnectorError::InvalidQuery(format!(
                "usage filters are not supported for metric code '{}' (handled by a bespoke aggregation)",
                params.code
            ))));
        }
        if !params.filters.is_empty() && self.match_extension(&params).is_some() {
            return Err(Report::new(ConnectorError::InvalidQuery(format!(
                "property filters are not supported for metric code '{}' (handled by a bespoke aggregation)",
                params.code
            ))));
        }

        let extension_query = match self.match_extension(&params) {
            Some(ext) => ext
                .build_query(&params)
                .map_err(ConnectorError::InvalidQuery)?,
            None => None,
        };
        let ch_query = match extension_query {
            Some(safe_query) => {
                tracing::debug!("Generated query (extension): {}", safe_query.sql);
                safe_query.into_query(&self.client)
            }
            None => {
                let safe_query =
                    sql::query_raw::query_meter_sql(params.clone(), &self.events_table)
                        .map_err(ConnectorError::InvalidQuery)?;
                tracing::debug!("Generated query: {}", safe_query.sql);
                safe_query.into_query(&self.client)
            }
        };

        let mut lines = ch_query
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)
            .attach("Failed to execute query with JSONEachRow")?
            .lines();

        let mut parsed = Vec::new();

        while let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value = serde_json::from_str(&line)
                .change_context(ConnectorError::QueryError)
                .attach("Failed to parse JSON row")?;

            let window_start = row
                .get_timestamp_utc("window_start")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing window_start field")?;

            let window_end = row
                .get_timestamp_utc("window_end")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing window_end field")?;
            let value = row
                .get_f64("value")
                .ok_or(ConnectorError::QueryError)
                .attach("Missing value field")?;

            let customer_id = if !params.groups_by_customer() {
                None
            } else {
                Some(
                    row.get_id("customer_id")
                        .ok_or(ConnectorError::QueryError)
                        .attach("Missing customer_id field")?,
                )
            };

            let mut group_by: HashMap<String, Option<String>> = HashMap::new();

            for column_name in &params.group_by {
                let col = PropertyColumn::from_str_ref(column_name);
                let column_value: Option<String> = row.get_string(&col.as_alias());
                group_by.insert(column_name.clone(), column_value);
            }

            if let Some(ref segmentation) = params.segmentation_filter
                && !params.merge_segments
            {
                match segmentation {
                    crate::domain::SegmentationFilter::Independent(filters) => {
                        for (column_name, _) in filters {
                            let col = PropertyColumn::from_str_ref(column_name);
                            let column_value: Option<String> = row.get_string(&col.as_alias());
                            group_by.insert(column_name.clone(), column_value);
                        }
                    }
                    crate::domain::SegmentationFilter::Linked {
                        dimension1_key,
                        dimension2_key,
                        ..
                    } => {
                        let col1 = PropertyColumn::from_str_ref(dimension1_key);
                        let col2 = PropertyColumn::from_str_ref(dimension2_key);
                        let dim1_value: Option<String> = row.get_string(col1.as_alias().as_str());
                        let dim2_value: Option<String> = row.get_string(col2.as_alias().as_str());
                        group_by.insert(dimension1_key.clone(), dim1_value);
                        group_by.insert(dimension2_key.clone(), dim2_value);
                    }
                }
            }

            parsed.push(Usage {
                window_start,
                window_end,
                value,
                customer_id,
                group_by,
            });
        }

        Ok(parsed)
    }

    #[tracing::instrument(skip_all)]
    async fn query_raw_events(
        &self,
        params: QueryRawEventsParams,
    ) -> Result<QueryRawEventsResult, Report<ConnectorError>> {
        let safe_query = sql::query_raw::query_raw_events_sql(params.clone(), &self.events_table)
            .map_err(ConnectorError::InvalidQuery)?;

        let rows = safe_query
            .into_query(&self.client)
            .fetch_all::<crate::ingest::domain::RawEventRow>()
            .await
            .change_context(ConnectorError::QueryError)?;

        let events = rows
            .into_iter()
            .map(|row| crate::ingest::domain::RawEvent {
                id: row.id,
                code: row.code,
                customer_id: CustomerId::from(row.customer_id),
                tenant_id: TenantId::from(row.tenant_id),
                timestamp: row.timestamp.naive_utc(),
                ingested_at: row.ingested_at.naive_utc(),
                properties: row.properties,
                keep_last: false,
            })
            .collect();

        Ok(QueryRawEventsResult { events })
    }

    #[tracing::instrument(skip(self))]
    async fn purge_tenants(&self, tenant_ids: &[TenantId]) -> Result<(), Report<ConnectorError>> {
        if tenant_ids.is_empty() {
            return Ok(());
        }

        // Mutations target the local replicated table (not the Distributed one),
        // one batched mutation for the whole set.
        let ids = tenant_ids
            .iter()
            .map(|id| format!("toUUID('{}')", id.as_uuid()))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            "ALTER TABLE {}.{} ON CLUSTER '{}' DELETE WHERE tenant_id IN ({})",
            self.database, RAW_EVENTS_LOCAL_TABLE, self.cluster_name, ids
        );

        self.client
            .query(&sql)
            .execute()
            .await
            .change_context(ConnectorError::QueryError)
            .attach("Failed to purge tenants raw events")?;

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn list_tenant_first_events(
        &self,
        tenants: &[TenantEventScan],
    ) -> Result<Vec<TenantFirstEvent>, Report<ConnectorError>> {
        #[derive(serde::Deserialize, clickhouse::Row)]
        struct FirstEventRow {
            #[serde(with = "clickhouse::serde::uuid")]
            tenant_id: uuid::Uuid,
            #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
            first_ingested_at: chrono::DateTime<chrono::Utc>,
        }

        // Chunked to stay under ClickHouse's max_query_size.
        const TENANTS_PER_QUERY: usize = 1000;
        let filters: Vec<String> = if tenants.is_empty() {
            vec![String::new()]
        } else {
            tenants
                .chunks(TENANTS_PER_QUERY)
                .map(|chunk| format!(" WHERE {}", tenant_filter(chunk)))
                .collect()
        };

        let mut out = Vec::new();
        for filter in filters {
            let sql = format!(
                "SELECT tenant_id, MIN(ingested_at) AS first_ingested_at FROM {}{} GROUP BY tenant_id",
                self.events_table, filter
            );

            let rows = self
                .client
                .query(&sql)
                .fetch_all::<FirstEventRow>()
                .await
                .change_context(ConnectorError::QueryError)
                .attach("Failed to list tenant first events")?;

            out.extend(rows.into_iter().map(|row| TenantFirstEvent {
                tenant_id: TenantId::from(row.tenant_id),
                first_ingested_at: row.first_ingested_at,
            }));
        }
        Ok(out)
    }

    #[tracing::instrument(skip(self))]
    async fn list_customer_first_events(
        &self,
        tenant_id: &TenantId,
        customer_ids: &[CustomerId],
    ) -> Result<Vec<CustomerFirstEvent>, Report<ConnectorError>> {
        #[derive(serde::Deserialize, clickhouse::Row)]
        struct FirstEventRow {
            #[serde(with = "clickhouse::serde::uuid")]
            customer_id: uuid::Uuid,
            #[serde(with = "clickhouse::serde::chrono::datetime64::nanos")]
            first_ingested_at: chrono::DateTime<chrono::Utc>,
        }

        // Chunked to stay under ClickHouse's max_query_size.
        const CUSTOMERS_PER_QUERY: usize = 1000;

        let mut out = Vec::new();
        for chunk in customer_ids.chunks(CUSTOMERS_PER_QUERY) {
            let ids = chunk
                .iter()
                .map(|id| format!("toUUID('{}')", id.as_uuid()))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT customer_id, MIN(ingested_at) AS first_ingested_at FROM {} \
                 WHERE tenant_id = toUUID('{}') AND customer_id IN ({}) GROUP BY customer_id",
                self.events_table,
                tenant_id.as_uuid(),
                ids
            );

            let rows = self
                .client
                .query(&sql)
                .fetch_all::<FirstEventRow>()
                .await
                .change_context(ConnectorError::QueryError)
                .attach("Failed to list customer first events")?;

            out.extend(rows.into_iter().map(|row| CustomerFirstEvent {
                customer_id: CustomerId::from(row.customer_id),
                first_ingested_at: row.first_ingested_at,
            }));
        }
        Ok(out)
    }

    #[tracing::instrument(skip(self))]
    async fn poll_usage_activity(
        &self,
        params: UsageActivityParams,
    ) -> Result<Vec<CustomerActivity>, Report<ConnectorError>> {
        #[derive(serde::Deserialize, clickhouse::Row)]
        struct ActivityRow {
            #[serde(with = "clickhouse::serde::uuid")]
            tenant_id: uuid::Uuid,
            #[serde(with = "clickhouse::serde::uuid")]
            customer_id: uuid::Uuid,
            #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
            last_seen_at: chrono::DateTime<chrono::Utc>,
            #[serde(with = "clickhouse::serde::chrono::datetime64::millis")]
            min_event_at: chrono::DateTime<chrono::Utc>,
        }

        if params.shard_count == 0 || params.shard_index >= params.shard_count {
            return Err(Report::new(ConnectorError::InvalidQuery(format!(
                "shard {} out of range for {} shards",
                params.shard_index, params.shard_count
            ))));
        }

        let mut query = self
            .client
            .query(&activity_sql(params.after.is_some()))
            .bind(params.since.timestamp_millis())
            .bind(params.shard_count)
            .bind(params.shard_index);
        if let Some(after) = &params.after {
            query = query
                .bind(after.last_seen_at.timestamp_millis())
                .bind(after.tenant_id.as_uuid().to_string())
                .bind(after.customer_id.as_uuid().to_string());
        }
        let rows = query
            .bind(params.limit)
            .fetch_all::<ActivityRow>()
            .await
            .change_context(ConnectorError::QueryError)
            .attach("Failed to poll usage activity")?;

        Ok(rows
            .into_iter()
            .map(|row| CustomerActivity {
                tenant_id: TenantId::from(row.tenant_id),
                customer_id: CustomerId::from(row.customer_id),
                last_seen_at: row.last_seen_at,
                min_event_at: row.min_event_at,
            })
            .collect())
    }

    async fn discover_event_codes(
        &self,
        tenant_id: &TenantId,
        search: &str,
        limit: u32,
    ) -> Result<Vec<EventCodeCount>, Report<ConnectorError>> {
        discovery::discover_event_codes(
            &self.client,
            &self.events_table,
            tenant_id.as_uuid(),
            search,
            limit,
        )
        .await
    }

    async fn discover_properties(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        lookback_days: u32,
        limit: u32,
    ) -> Result<Vec<DiscoveredProperty>, Report<ConnectorError>> {
        discovery::discover_properties(
            &self.client,
            &self.events_table,
            tenant_id.as_uuid(),
            event_code,
            lookback_days,
            limit,
        )
        .await
    }

    async fn discover_property_values(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        property_key: &str,
        search: &str,
        limit: u32,
    ) -> Result<Vec<PropertyValueCount>, Report<ConnectorError>> {
        discovery::discover_property_values(
            &self.client,
            &self.events_table,
            tenant_id.as_uuid(),
            event_code,
            property_key,
            search,
            limit,
        )
        .await
    }

    async fn profile_event_properties(
        &self,
        tenant_id: &TenantId,
        event_code: &str,
        from_ts: i64,
        to_ts: i64,
        top_values: u32,
        limit: u32,
    ) -> Result<EventPropertiesProfile, Report<ConnectorError>> {
        discovery::profile_event_properties(
            &self.client,
            &self.events_table,
            tenant_id.as_uuid(),
            event_code,
            from_ts,
            to_ts,
            top_values,
            limit,
        )
        .await
    }
}

// The shard is the last byte of the tenant id, as on the Postgres side that lists stale customers.
// A page resumes strictly after the previous page's last row, so a full page cannot be replayed.
fn activity_sql(paged: bool) -> String {
    let after = if paged {
        "HAVING (last_seen_at, tenant_id, customer_id) > \
         (fromUnixTimestamp64Milli(?), toUUID(?), toUUID(?)) "
    } else {
        ""
    };
    format!(
        "SELECT tenant_id, customer_id, max(seen_at) AS last_seen_at, \
         min(min_event_at) AS min_event_at FROM usage_activity \
         WHERE seen_at > fromUnixTimestamp64Milli(?) \
         AND reinterpretAsUInt8(unhex(right(toString(tenant_id), 2))) % ? = ? \
         GROUP BY tenant_id, customer_id {after}\
         ORDER BY last_seen_at ASC, tenant_id, customer_id LIMIT ?"
    )
}

async fn supports_setting(client: &Client, name: &str) -> Result<bool, Report<ConnectorError>> {
    let count = client
        .query("SELECT count() FROM system.settings WHERE name = ?")
        .bind(name)
        .fetch_one::<u64>()
        .await
        .change_context(ConnectorError::InitError(format!(
            "Failed to check support for ClickHouse setting {name}"
        )))?;
    Ok(count > 0)
}

/// Unbounded tenants collapse into one `IN` list; bounded ones each get their
/// own `ingested_at` floor so a sandbox's seeded events stay out of the scan.
fn tenant_filter(chunk: &[TenantEventScan]) -> String {
    let mut terms = Vec::new();
    let unbounded: Vec<String> = chunk
        .iter()
        .filter(|t| t.from.is_none())
        .map(|t| format!("toUUID('{}')", t.tenant_id.as_uuid()))
        .collect();
    if !unbounded.is_empty() {
        terms.push(format!("tenant_id IN ({})", unbounded.join(",")));
    }
    for tenant in chunk.iter().filter(|t| t.from.is_some()) {
        let from = tenant.from.expect("filtered to Some");
        terms.push(format!(
            "(tenant_id = toUUID('{}') AND ingested_at >= toDateTime64('{}', 9, 'UTC'))",
            tenant.tenant_id.as_uuid(),
            from.format("%Y-%m-%d %H:%M:%S%.9f")
        ));
    }
    terms.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[tokio::test]
    async fn supports_setting_fails_when_clickhouse_is_unreachable() {
        let client = Client::default().with_url("http://127.0.0.1:1");
        assert!(
            supports_setting(&client, LAZY_MATERIALIZATION)
                .await
                .is_err()
        );
    }

    #[test]
    fn tenant_filter_bounds_only_the_tenants_that_ask_for_it() {
        let unbounded = TenantId::from(uuid::Uuid::nil());
        let bounded = TenantId::from(uuid::uuid!("00000000-0000-0000-0000-0000000000a3"));
        let filter = tenant_filter(&[
            TenantEventScan {
                tenant_id: unbounded,
                from: None,
            },
            TenantEventScan {
                tenant_id: bounded,
                from: Some(chrono::Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap()),
            },
        ]);
        assert_eq!(
            filter,
            "tenant_id IN (toUUID('00000000-0000-0000-0000-000000000000')) OR \
             (tenant_id = toUUID('00000000-0000-0000-0000-0000000000a3') AND \
             ingested_at >= toDateTime64('2026-01-05 00:00:00.000000000', 9, 'UTC'))"
        );
    }
}
