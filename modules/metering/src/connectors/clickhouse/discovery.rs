//! Raw-event introspection for the billable-metric editor. The event code, property key and
//! search are bound; `tenant_id` is a UUID and the numeric bounds are typed, so nothing
//! client-supplied reaches SQL as an identifier.

use std::collections::HashMap;

use clickhouse::Client;
use error_stack::{Report, ResultExt};
use tokio::io::AsyncBufReadExt;

use crate::connectors::errors::ConnectorError;
use crate::connectors::json::JsonFieldExtractor;
use crate::domain::{
    DiscoveredProperty, EventCodeCount, EventPropertiesProfile, PropertyNumericStats,
    PropertyProfile, PropertyValueCount,
};

/// Each key with its value cardinality and a small sample, most frequent key first.
#[tracing::instrument(skip(client))]
pub(super) async fn discover_properties(
    client: &Client,
    events_table: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    lookback_days: u32,
    limit: u32,
) -> Result<Vec<DiscoveredProperty>, Report<ConnectorError>> {
    let sql = format!(
        r#"
        SELECT
          key                          AS key,
          toUInt64(uniqExact(val))     AS cardinality,
          topK(8)(val)                 AS samples,
          toUInt64(countIf(val != '' AND isNotNull(toFloat64OrNull(val)))) AS numeric_count,
          toUInt64(countIf(val != '')) AS nonempty_count
        FROM {table}
        ARRAY JOIN mapKeys(properties) AS key, mapValues(properties) AS val
        WHERE tenant_id = toUUID('{tenant}')
          AND code = ?
          AND timestamp >= now() - INTERVAL {days} DAY
        GROUP BY key
        ORDER BY count() DESC
        LIMIT {limit}
        "#,
        table = events_table,
        tenant = tenant_id,
        days = lookback_days,
        limit = limit,
    );

    let mut lines = client
        .query(&sql)
        .bind(event_code)
        .with_setting("max_execution_time", "8")
        .with_setting("max_memory_usage", "536870912")
        .with_setting("max_bytes_before_external_group_by", "268435456")
        .with_setting("output_format_json_quote_64bit_integers", "0")
        .fetch_bytes("JSONEachRow")
        .change_context(ConnectorError::QueryError)
        .attach("Failed to execute property discovery query")?
        .lines();

    let mut out = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .change_context(ConnectorError::QueryError)?
    {
        let row: serde_json::Value = serde_json::from_str(&line)
            .change_context(ConnectorError::QueryError)
            .attach("Failed to parse discovery JSON row")?;

        let Some(key) = row.get_string("key").filter(|k| !k.is_empty()) else {
            continue;
        };
        let cardinality = row.get_f64("cardinality").unwrap_or(0.0) as u64;
        let sample_values = row
            .get("samples")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let numeric_count = row.get_f64("numeric_count").unwrap_or(0.0);
        let nonempty_count = row.get_f64("nonempty_count").unwrap_or(0.0);
        let numeric_ratio = if nonempty_count > 0.0 {
            numeric_count / nonempty_count
        } else {
            0.0
        };

        out.push(DiscoveredProperty {
            key,
            cardinality,
            sample_values,
            numeric_ratio,
        });
    }

    Ok(out)
}

/// Reads the same events table a metric over this code aggregates, so the profile matches
/// what the metric would see.
#[tracing::instrument(skip(client))]
pub(super) async fn profile_event_properties(
    client: &Client,
    events_table: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    from_ts: i64,
    to_ts: i64,
    top_values: u32,
    limit: u32,
) -> Result<EventPropertiesProfile, Report<ConnectorError>> {
    // Fraction of non-empty values that must parse as numbers for a key to be
    // treated as numeric (and get distribution stats rather than a value list).
    const NUMERIC_THRESHOLD: f64 = 0.8;

    let from_expr = if from_ts > 0 {
        format!("toDateTime({from_ts})")
    } else {
        "now() - INTERVAL 30 DAY".to_string()
    };
    let to_expr = if to_ts > 0 {
        format!("toDateTime({to_ts})")
    } else {
        "now()".to_string()
    };
    let time_where = format!("timestamp >= {from_expr} AND timestamp < {to_expr}",);

    // (1) Coverage denominator: total events of this code in range.
    let total_sql = format!(
        r#"
        SELECT toUInt64(count()) AS total
        FROM {table}
        WHERE tenant_id = toUUID('{tenant}') AND code = ? AND {time_where}
        "#,
        table = events_table,
        tenant = tenant_id,
    );
    let total_events = {
        let mut lines = client
            .query(&total_sql)
            .bind(event_code)
            .with_setting("max_execution_time", "8")
            .with_setting("max_memory_usage", "536870912")
            .with_setting("max_bytes_before_external_group_by", "268435456")
            .with_setting("output_format_json_quote_64bit_integers", "0")
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)
            .attach("Failed to execute property profile count query")?
            .lines();
        let mut total = 0u64;
        if let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value =
                serde_json::from_str(&line).change_context(ConnectorError::QueryError)?;
            total = row.get_f64("total").unwrap_or(0.0) as u64;
        }
        total
    };

    // (2) Per-key stats: coverage, cardinality, numeric ratio + distribution.
    let stats_sql = format!(
        r#"
        SELECT
          key                                                       AS key,
          toUInt64(count())                                         AS event_count,
          toUInt64(uniqExact(val))                                  AS cardinality,
          toUInt64(countIf(val != '' AND isNotNull(toFloat64OrNull(val)))) AS numeric_count,
          toUInt64(countIf(val != ''))                              AS nonempty_count,
          min(toFloat64OrNull(val))                                 AS num_min,
          max(toFloat64OrNull(val))                                 AS num_max,
          avg(toFloat64OrNull(val))                                 AS num_avg,
          sum(toFloat64OrNull(val))                                 AS num_sum,
          quantile(0.5)(toFloat64OrNull(val))                       AS num_p50,
          quantile(0.95)(toFloat64OrNull(val))                      AS num_p95
        FROM {table}
        ARRAY JOIN mapKeys(properties) AS key, mapValues(properties) AS val
        WHERE tenant_id = toUUID('{tenant}') AND code = ? AND {time_where}
        GROUP BY key
        ORDER BY event_count DESC
        LIMIT {limit}
        "#,
        table = events_table,
        tenant = tenant_id,
        limit = limit,
    );
    let mut profiles: Vec<PropertyProfile> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    {
        let mut lines = client
            .query(&stats_sql)
            .bind(event_code)
            .with_setting("max_execution_time", "8")
            .with_setting("max_memory_usage", "536870912")
            .with_setting("max_bytes_before_external_group_by", "268435456")
            .with_setting("output_format_json_quote_64bit_integers", "0")
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)
            .attach("Failed to execute property profile stats query")?
            .lines();
        while let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value = serde_json::from_str(&line)
                .change_context(ConnectorError::QueryError)
                .attach("Failed to parse property profile JSON row")?;
            let Some(key) = row.get_string("key").filter(|k| !k.is_empty()) else {
                continue;
            };
            let event_count = row.get_f64("event_count").unwrap_or(0.0) as u64;
            let cardinality = row.get_f64("cardinality").unwrap_or(0.0) as u64;
            let numeric_count = row.get_f64("numeric_count").unwrap_or(0.0);
            let nonempty_count = row.get_f64("nonempty_count").unwrap_or(0.0);
            let numeric_ratio = if nonempty_count > 0.0 {
                numeric_count / nonempty_count
            } else {
                0.0
            };
            let numeric = if numeric_ratio >= NUMERIC_THRESHOLD && numeric_count > 0.0 {
                Some(PropertyNumericStats {
                    min: row.get_f64("num_min").unwrap_or(0.0),
                    max: row.get_f64("num_max").unwrap_or(0.0),
                    avg: row.get_f64("num_avg").unwrap_or(0.0),
                    sum: row.get_f64("num_sum").unwrap_or(0.0),
                    p50: row.get_f64("num_p50").unwrap_or(0.0),
                    p95: row.get_f64("num_p95").unwrap_or(0.0),
                })
            } else {
                None
            };
            let coverage_ratio = if total_events > 0 {
                event_count as f64 / total_events as f64
            } else {
                0.0
            };
            index.insert(key.clone(), profiles.len());
            profiles.push(PropertyProfile {
                key,
                event_count,
                coverage_ratio,
                cardinality,
                numeric_ratio,
                numeric,
                top_values: Vec::new(),
            });
        }
    }

    // (3) Top-N values per key with counts (only for the keys kept above).
    if !profiles.is_empty() && top_values > 0 {
        let values_sql = format!(
            r#"
            SELECT key AS key, val AS value, toUInt64(count()) AS cnt
            FROM {table}
            ARRAY JOIN mapKeys(properties) AS key, mapValues(properties) AS val
            WHERE tenant_id = toUUID('{tenant}') AND code = ? AND val != '' AND {time_where}
            GROUP BY key, value
            ORDER BY key, cnt DESC
            LIMIT {top_values} BY key
            "#,
            table = events_table,
            tenant = tenant_id,
            top_values = top_values,
        );
        let mut lines = client
            .query(&values_sql)
            .bind(event_code)
            .with_setting("max_execution_time", "8")
            .with_setting("max_memory_usage", "536870912")
            .with_setting("max_bytes_before_external_group_by", "268435456")
            .with_setting("output_format_json_quote_64bit_integers", "0")
            .fetch_bytes("JSONEachRow")
            .change_context(ConnectorError::QueryError)
            .attach("Failed to execute property profile values query")?
            .lines();
        while let Some(line) = lines
            .next_line()
            .await
            .change_context(ConnectorError::QueryError)?
        {
            let row: serde_json::Value =
                serde_json::from_str(&line).change_context(ConnectorError::QueryError)?;
            let (Some(key), Some(value)) = (row.get_string("key"), row.get_string("value")) else {
                continue;
            };
            if let Some(&i) = index.get(&key) {
                profiles[i].top_values.push(PropertyValueCount {
                    value,
                    count: row.get_f64("cnt").unwrap_or(0.0) as u64,
                });
            }
        }
    }

    Ok(EventPropertiesProfile {
        total_events,
        properties: profiles,
    })
}

/// Over the trailing 90 days; `search` is a case-insensitive substring.
#[tracing::instrument(skip(client))]
pub(super) async fn discover_event_codes(
    client: &Client,
    events_table: &str,
    tenant_id: uuid::Uuid,
    search: &str,
    limit: u32,
) -> Result<Vec<EventCodeCount>, Report<ConnectorError>> {
    let search_frag = if search.is_empty() {
        ""
    } else {
        "AND code ILIKE ?"
    };
    let sql = format!(
        r#"
        SELECT code AS code, toUInt64(count()) AS cnt
        FROM {table}
        WHERE tenant_id = toUUID('{tenant}')
          AND timestamp >= now() - INTERVAL 90 DAY
          {search_frag}
        GROUP BY code
        ORDER BY cnt DESC
        LIMIT {limit}
        "#,
        table = events_table,
        tenant = tenant_id,
        limit = limit,
    );

    let mut query = client.query(&sql);
    if !search.is_empty() {
        query = query.bind(format!("%{search}%"));
    }
    let mut lines = query
        .with_setting("max_execution_time", "8")
        .with_setting("max_memory_usage", "536870912")
        .with_setting("max_bytes_before_external_group_by", "268435456")
        .with_setting("output_format_json_quote_64bit_integers", "0")
        .fetch_bytes("JSONEachRow")
        .change_context(ConnectorError::QueryError)
        .attach("Failed to execute event-code discovery query")?
        .lines();

    let mut out = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .change_context(ConnectorError::QueryError)?
    {
        let row: serde_json::Value = serde_json::from_str(&line)
            .change_context(ConnectorError::QueryError)
            .attach("Failed to parse event-code discovery JSON row")?;
        let Some(code) = row.get_string("code").filter(|c| !c.is_empty()) else {
            continue;
        };
        out.push(EventCodeCount {
            code,
            count: row.get_f64("cnt").unwrap_or(0.0) as u64,
        });
    }

    Ok(out)
}

/// Over the trailing 90 days; binds `event_code`, `property_key`, then `search` (if any).
#[tracing::instrument(skip(client))]
pub(super) async fn discover_property_values(
    client: &Client,
    events_table: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    property_key: &str,
    search: &str,
    limit: u32,
) -> Result<Vec<PropertyValueCount>, Report<ConnectorError>> {
    let search_frag = if search.is_empty() {
        ""
    } else {
        "AND val ILIKE ?"
    };
    let sql = format!(
        r#"
        SELECT val AS value, toUInt64(count()) AS cnt
        FROM {table}
        ARRAY JOIN mapKeys(properties) AS key, mapValues(properties) AS val
        WHERE tenant_id = toUUID('{tenant}')
          AND code = ?
          AND key = ?
          AND val != ''
          AND timestamp >= now() - INTERVAL 90 DAY
          {search_frag}
        GROUP BY value
        ORDER BY cnt DESC
        LIMIT {limit}
        "#,
        table = events_table,
        tenant = tenant_id,
        limit = limit,
    );

    let mut query = client.query(&sql).bind(event_code).bind(property_key);
    if !search.is_empty() {
        query = query.bind(format!("%{search}%"));
    }
    let mut lines = query
        .with_setting("max_execution_time", "8")
        .with_setting("max_memory_usage", "536870912")
        .with_setting("max_bytes_before_external_group_by", "268435456")
        .with_setting("output_format_json_quote_64bit_integers", "0")
        .fetch_bytes("JSONEachRow")
        .change_context(ConnectorError::QueryError)
        .attach("Failed to execute property-value discovery query")?
        .lines();

    let mut out = Vec::new();
    while let Some(line) = lines
        .next_line()
        .await
        .change_context(ConnectorError::QueryError)?
    {
        let row: serde_json::Value = serde_json::from_str(&line)
            .change_context(ConnectorError::QueryError)
            .attach("Failed to parse property-value discovery JSON row")?;
        let Some(value) = row.get_string("value") else {
            continue;
        };
        out.push(PropertyValueCount {
            value,
            count: row.get_f64("cnt").unwrap_or(0.0) as u64,
        });
    }

    Ok(out)
}
