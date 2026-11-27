//! Raw-event introspection for the billable-metric editor. The event code, property key and
//! search are bound; `tenant_id` is a UUID and the numeric bounds are typed, so nothing
//! client-supplied reaches SQL as an identifier.

use std::collections::HashMap;

use clickhouse::Client;
use error_stack::{Report, ResultExt};
use metric_core::sql::ast::{Expr, Ident, Item, Op, Order, Query, Relation, Select};
use metric_core::sql::{BindValue, EventDedup, EventsTable, VoidScope, render_scoped};
use tokio::io::AsyncBufReadExt;

use crate::connectors::errors::ConnectorError;
use crate::connectors::json::JsonFieldExtractor;
use crate::domain::{
    DiscoveredProperty, EventCodeCount, EventPropertiesProfile, PropertyNumericStats,
    PropertyProfile, PropertyValueCount,
};

const RECENT: &str = "now() - INTERVAL 90 DAY";

fn bind_str(value: &str) -> Expr {
    Expr::Bind(BindValue::String(value.to_string()))
}

/// `now() - INTERVAL <days> DAY`.
fn days_ago(days: u32) -> Expr {
    Expr::call("now", []).op(Op::Sub, Expr::Interval(days.into(), "DAY"))
}

/// The tenant's live events of `columns` (ADR-0012 §6); these reads have no keep-last flag.
fn live_events(
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    columns: &[&'static str],
    filter: Vec<Expr>,
    scope: VoidScope,
) -> Relation {
    let columns = columns.iter().map(|c| Ident::trusted(c)).collect();
    events
        .source(database, tenant_id, EventDedup::Off)
        .read(columns, filter, scope)
}

/// `read` as a client query, once its tenant scope is checked, under discovery's resource limits.
fn scoped(
    client: &Client,
    read: &Query,
    tenant_id: uuid::Uuid,
) -> Result<clickhouse::query::Query, Report<ConnectorError>> {
    let read = render_scoped(read, &[], tenant_id)
        .map_err(|e| Report::new(ConnectorError::InvalidQuery(e)))?;
    Ok(read
        .into_query(client)
        .with_setting("max_execution_time", "8")
        .with_setting("max_memory_usage", "536870912")
        .with_setting("max_bytes_before_external_group_by", "268435456")
        .with_setting("output_format_json_quote_64bit_integers", "0"))
}

/// Each event of `source`, once per property: `key` and `val`.
fn per_property(source: Relation) -> Relation {
    Relation::ArrayJoin {
        from: Box::new(source),
        columns: vec![
            (
                Expr::call("mapKeys", [Expr::col("properties")]),
                Ident::trusted("key"),
            ),
            (
                Expr::call("mapValues", [Expr::col("properties")]),
                Ident::trusted("val"),
            ),
        ],
    }
}

fn count() -> Expr {
    Expr::call("toUInt64", [Expr::call("count", [])])
}

fn non_empty() -> Expr {
    Expr::col("val").ne(Expr::str(""))
}

/// The value counts a key's numeric ratio is made of: values that parse, over non-empty ones.
fn numeric_counts() -> [Item; 2] {
    let numeric = Expr::And(vec![
        non_empty(),
        Expr::call(
            "isNotNull",
            [Expr::call("toFloat64OrNull", [Expr::col("val")])],
        ),
    ]);
    [
        Item::as_(
            Expr::call("toUInt64", [Expr::call("countIf", [numeric])]),
            "numeric_count",
        ),
        Item::as_(
            Expr::call("toUInt64", [Expr::call("countIf", [non_empty()])]),
            "nonempty_count",
        ),
    ]
}

fn tenant_is(tenant_id: uuid::Uuid) -> Expr {
    Expr::col("tenant_id").eq(Expr::Uuid(tenant_id))
}

pub(super) fn properties_query(
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    lookback_days: u32,
    limit: u32,
) -> Query {
    let since = days_ago(lookback_days);
    let source = live_events(
        events,
        database,
        tenant_id,
        &["properties"],
        vec![
            tenant_is(tenant_id),
            Expr::col("code").eq(bind_str(event_code)),
            Expr::col("timestamp").ge(since),
        ],
        VoidScope::default()
            .code(event_code)
            .since(format!("now() - INTERVAL {lookback_days} DAY")),
    );
    let mut items = vec![
        Item::as_(Expr::col("key"), "key"),
        Item::as_(
            Expr::call("toUInt64", [Expr::call("uniqExact", [Expr::col("val")])]),
            "cardinality",
        ),
        Item::as_(
            Expr::Parametric("topK", vec![Expr::Int(8)], vec![Expr::col("val")]),
            "samples",
        ),
    ];
    items.extend(numeric_counts());
    Select::new(items, per_property(source))
        .group_by(vec![Expr::col("key")])
        .order_by(vec![Order::desc(Expr::call("count", []))])
        .limit(limit.into())
        .query()
}

/// The events of `event_code` in `[from_ts, to_ts)`, epoch seconds, 0 = the trailing 30 days
/// ending now.
pub(super) fn profile_source(
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    from_ts: i64,
    to_ts: i64,
) -> Relation {
    let (from, from_sql) = if from_ts > 0 {
        (
            Expr::call("toDateTime", [Expr::Int(from_ts)]),
            format!("toDateTime({from_ts})"),
        )
    } else {
        (days_ago(30), "now() - INTERVAL 30 DAY".to_string())
    };
    let (to, to_sql) = if to_ts > 0 {
        (
            Expr::call("toDateTime", [Expr::Int(to_ts)]),
            format!("toDateTime({to_ts})"),
        )
    } else {
        (Expr::call("now", []), "now()".to_string())
    };
    live_events(
        events,
        database,
        tenant_id,
        &["properties"],
        vec![
            tenant_is(tenant_id),
            Expr::col("code").eq(bind_str(event_code)),
            Expr::col("timestamp").ge(from),
            Expr::col("timestamp").lt(to),
        ],
        VoidScope::default()
            .code(event_code)
            .since(from_sql)
            .until(to_sql),
    )
}

/// The coverage denominator: every event of the profile.
pub(super) fn profile_total_query(source: Relation) -> Query {
    Select::new(vec![Item::as_(count(), "total")], source).query()
}

/// Per key: coverage, cardinality, numeric ratio and distribution.
pub(super) fn profile_stats_query(source: Relation, limit: u32) -> Query {
    let number = || Expr::call("toFloat64OrNull", [Expr::col("val")]);
    let mut items = vec![
        Item::as_(Expr::col("key"), "key"),
        Item::as_(count(), "event_count"),
        Item::as_(
            Expr::call("toUInt64", [Expr::call("uniqExact", [Expr::col("val")])]),
            "cardinality",
        ),
    ];
    items.extend(numeric_counts());
    for (function, alias) in [
        ("min", "num_min"),
        ("max", "num_max"),
        ("avg", "num_avg"),
        ("sum", "num_sum"),
    ] {
        items.push(Item::as_(Expr::call(function, [number()]), alias));
    }
    for (level, alias) in [(0.5, "num_p50"), (0.95, "num_p95")] {
        items.push(Item::as_(
            Expr::Parametric("quantile", vec![Expr::Float(level)], vec![number()]),
            alias,
        ));
    }
    Select::new(items, per_property(source))
        .group_by(vec![Expr::col("key")])
        .order_by(vec![Order::desc(Expr::col("event_count"))])
        .limit(limit.into())
        .query()
}

/// The `top_values` most frequent values of each key.
pub(super) fn profile_values_query(source: Relation, top_values: u32) -> Query {
    Select::new(
        vec![
            Item::as_(Expr::col("key"), "key"),
            Item::as_(Expr::col("val"), "value"),
            Item::as_(count(), "cnt"),
        ],
        per_property(source),
    )
    .filter(vec![non_empty()])
    .group_by(vec![Expr::col("key"), Expr::col("value")])
    .order_by(vec![Expr::col("key").into(), Order::desc(Expr::col("cnt"))])
    .limit_by(top_values.into(), vec![Expr::col("key")])
    .query()
}

const RECENT_DAYS: u32 = 90;

pub(super) fn event_codes_query(
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    search: &str,
    limit: u32,
) -> Query {
    let mut filter = vec![
        tenant_is(tenant_id),
        Expr::col("timestamp").ge(days_ago(RECENT_DAYS)),
    ];
    if !search.is_empty() {
        filter.push(Expr::col("code").ilike(bind_str(&format!("%{search}%"))));
    }
    let source = live_events(
        events,
        database,
        tenant_id,
        &["code"],
        filter,
        VoidScope::default().since(RECENT),
    );
    Select::new(
        vec![
            Item::as_(Expr::col("code"), "code"),
            Item::as_(count(), "cnt"),
        ],
        source,
    )
    .group_by(vec![Expr::col("code")])
    .order_by(vec![Order::desc(Expr::col("cnt"))])
    .limit(limit.into())
    .query()
}

pub(super) fn property_values_query(
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    property_key: &str,
    search: &str,
    limit: u32,
) -> Query {
    let source = live_events(
        events,
        database,
        tenant_id,
        &["properties"],
        vec![
            tenant_is(tenant_id),
            Expr::col("code").eq(bind_str(event_code)),
            Expr::col("timestamp").ge(days_ago(RECENT_DAYS)),
        ],
        VoidScope::default().code(event_code).since(RECENT),
    );
    let mut filter = vec![Expr::col("key").eq(bind_str(property_key)), non_empty()];
    if !search.is_empty() {
        filter.push(Expr::col("val").ilike(bind_str(&format!("%{search}%"))));
    }
    Select::new(
        vec![
            Item::as_(Expr::col("val"), "value"),
            Item::as_(count(), "cnt"),
        ],
        per_property(source),
    )
    .filter(filter)
    .group_by(vec![Expr::col("value")])
    .order_by(vec![Order::desc(Expr::col("cnt"))])
    .limit(limit.into())
    .query()
}

/// Each key with its value cardinality and a small sample, most frequent key first.
#[tracing::instrument(skip(client))]
pub(super) async fn discover_properties(
    client: &Client,
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    lookback_days: u32,
    limit: u32,
) -> Result<Vec<DiscoveredProperty>, Report<ConnectorError>> {
    let read = properties_query(
        events,
        database,
        tenant_id,
        event_code,
        lookback_days,
        limit,
    );
    let mut lines = scoped(client, &read, tenant_id)?
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
    events: &EventsTable,
    database: &str,
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

    let source = profile_source(events, database, tenant_id, event_code, from_ts, to_ts);

    // (1) Coverage denominator: total events of this code in range.
    let total_events = {
        let mut lines = scoped(client, &profile_total_query(source.clone()), tenant_id)?
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
    let mut profiles: Vec<PropertyProfile> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    {
        let mut lines = scoped(
            client,
            &profile_stats_query(source.clone(), limit),
            tenant_id,
        )?
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
        let mut lines = scoped(client, &profile_values_query(source, top_values), tenant_id)?
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
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    search: &str,
    limit: u32,
) -> Result<Vec<EventCodeCount>, Report<ConnectorError>> {
    let read = event_codes_query(events, database, tenant_id, search, limit);
    let mut lines = scoped(client, &read, tenant_id)?
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
    events: &EventsTable,
    database: &str,
    tenant_id: uuid::Uuid,
    event_code: &str,
    property_key: &str,
    search: &str,
    limit: u32,
) -> Result<Vec<PropertyValueCount>, Report<ConnectorError>> {
    let read = property_values_query(
        events,
        database,
        tenant_id,
        event_code,
        property_key,
        search,
        limit,
    );
    let mut lines = scoped(client, &read, tenant_id)?
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> uuid::Uuid {
        uuid::Uuid::from_u128(0xd15c)
    }

    fn assert_read(name: &str, read: Query) {
        let rendered = render_scoped(&read, &[], tenant()).expect("tenant-scoped");
        assert_eq!(
            rendered.placeholders(),
            rendered.binds.len(),
            "{}",
            rendered.sql
        );
        insta::assert_snapshot!(name, metric_core::sql::render::pretty(&read, &[]), name);
    }

    #[test]
    fn discovery_reads() {
        let events: EventsTable = "raw_events_v2".into();
        let e = &events;
        assert_read(
            "properties",
            properties_query(e, "meteroid", tenant(), "api", 30, 50),
        );
        for (name, from, to) in [
            ("profile_window", 1_767_571_200, 1_769_904_000),
            ("profile_trailing", 0, 0),
        ] {
            let source = || profile_source(e, "meteroid", tenant(), "api", from, to);
            assert_read(&format!("{name}_total"), profile_total_query(source()));
            assert_read(&format!("{name}_stats"), profile_stats_query(source(), 40));
            assert_read(&format!("{name}_values"), profile_values_query(source(), 5));
        }
        assert_read(
            "event_codes",
            event_codes_query(e, "meteroid", tenant(), "", 20),
        );
        assert_read(
            "event_codes_search",
            event_codes_query(e, "meteroid", tenant(), "ap", 20),
        );
        assert_read(
            "property_values",
            property_values_query(e, "meteroid", tenant(), "api", "region", "", 20),
        );
        assert_read(
            "property_values_search",
            property_values_query(e, "meteroid", tenant(), "api", "region", "eu", 20),
        );
    }
}
