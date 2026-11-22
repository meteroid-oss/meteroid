use crate::connectors::clickhouse::sql::{BindValue, PropertyColumn, SafeQuery, meter_value};
use crate::domain::{
    EventAttribution, EventSortOrder, MeterAggregation, OriginFilter, PropertyFilterOp,
    QueryMeterParams, QueryRawEventsParams, SegmentationFilter, WindowSize,
};
use chrono::Utc;
use common_domain::ids::BaseId;
use metric_core::sql::{EventDedup, EventsTable, VoidScope};

/// Append pre-aggregation property filters as WHERE conditions (ANDed), with binds pushed
/// in lock-step. `In` keeps matching events; `NotIn` is presence-required (mapContains guards
/// an absent key from passing `!=`). Shared by the meter and raw-event queries so the events
/// explorer filters identically to how billing meters.
fn push_property_filters(
    filters: &[crate::domain::PropertyFilter],
    conditions: &mut Vec<String>,
    binds: &mut Vec<BindValue>,
) {
    for f in filters {
        if f.values.is_empty() {
            continue;
        }
        let col = PropertyColumn(&f.property);
        match f.op {
            PropertyFilterOp::In => {
                let ored = f
                    .values
                    .iter()
                    .map(|value| {
                        let path = col.path_sql(binds);
                        binds.push(BindValue::String(value.clone()));
                        format!("{path} = ?")
                    })
                    .collect::<Vec<_>>()
                    .join(" OR ");
                conditions.push(format!("({ored})"));
            }
            PropertyFilterOp::NotIn => {
                binds.push(BindValue::String(f.property.clone()));
                let mut parts = vec!["mapContains(properties, ?)".to_string()];
                for value in &f.values {
                    let path = col.path_sql(binds);
                    binds.push(BindValue::String(value.clone()));
                    parts.push(format!("{path} != ?"));
                }
                conditions.push(format!("({})", parts.join(" AND ")));
            }
        }
    }
}

pub fn query_raw_events_sql(
    params: QueryRawEventsParams,
    events: &EventsTable,
    database: &str,
) -> Result<SafeQuery, String> {
    let source = events.source(
        database,
        params.tenant_id.as_uuid(),
        EventDedup::new(params.keep_last_events),
    );
    let mut conditions = Vec::new();
    let mut binds: Vec<BindValue> = Vec::new();

    conditions.push("tenant_id = ?".to_string());
    binds.push(BindValue::Uuid(params.tenant_id.as_uuid()));

    conditions.push("timestamp >= toDateTime(?)".to_string());
    binds.push(BindValue::I64(params.from.timestamp()));

    let to = params.to.unwrap_or_else(Utc::now).timestamp();
    conditions.push("timestamp < toDateTime(?)".to_string());
    binds.push(BindValue::I64(to));

    let customers: Vec<_> = params.customer_ids.iter().map(|id| id.as_uuid()).collect();
    let scope = VoidScope::default()
        .codes(&params.event_codes)
        .customers(customers.clone())
        .since(format!("toDateTime({})", params.from.timestamp()))
        .until(format!("toDateTime({to})"));

    if !customers.is_empty() {
        conditions.push("customer_id IN ?".to_string());
        binds.push(BindValue::Uuids(customers));
    }

    if !params.event_codes.is_empty() {
        conditions.push("code IN ?".to_string());
        binds.push(BindValue::Strings(params.event_codes));
    }

    if let Some(search) = params.search {
        let pattern = format!("%{search}%");
        conditions.push(
            "(id ILIKE ? OR code ILIKE ? OR arrayStringConcat(mapValues(properties), ' ') ILIKE ?)"
                .to_string(),
        );
        binds.push(BindValue::String(pattern.clone()));
        binds.push(BindValue::String(pattern.clone()));
        binds.push(BindValue::String(pattern));
    }

    push_property_filters(&params.filters, &mut conditions, &mut binds);

    let (order_by, keyset_cmp) = match params.sort_order {
        EventSortOrder::TimestampDesc => ("timestamp DESC, id DESC", Some("<")),
        EventSortOrder::TimestampAsc => ("timestamp ASC, id ASC", Some(">")),
        EventSortOrder::IngestedDesc => ("ingested_at DESC", None),
        EventSortOrder::IngestedAsc => ("ingested_at ASC", None),
    };

    let keyset = match params.after {
        Some(after) => {
            let cmp = keyset_cmp
                .ok_or_else(|| "keyset pagination requires a timestamp sort order".to_string())?;
            binds.push(BindValue::I64(after.timestamp_nanos));
            binds.push(BindValue::I64(after.timestamp_nanos));
            binds.push(BindValue::String(after.id));
            Some(format!(
                "(toUnixTimestamp64Nano(timestamp) {cmp} ? OR (toUnixTimestamp64Nano(timestamp) = ? AND id {cmp} ?))"
            ))
        }
        None => None,
    };

    let columns = RAW_EVENT_COLUMNS.join(", ");
    let conditions = conditions.join(" AND ");
    // Without a dedup the query stays flat: ClickHouse 25.6 lazy materialization segfaults on
    // `ORDER BY .. LIMIT n` over a bare subquery of a Distributed table.
    let sql = match source.dedup() {
        EventDedup::Off => {
            let from = source.flat_from_sql(&conditions, &scope);
            let keyset = keyset.map(|k| format!(" AND {k}")).unwrap_or_default();
            format!("SELECT {columns} {from}{keyset} ORDER BY {order_by} LIMIT ? OFFSET ?")
        }
        EventDedup::LastIngested => {
            let from = source.from_sql(&columns, &conditions, &scope);
            let outer_where = keyset
                .map(|keyset| format!(" WHERE {keyset}"))
                .unwrap_or_default();
            format!("SELECT {columns} {from}{outer_where} ORDER BY {order_by} LIMIT ? OFFSET ?")
        }
    };
    binds.push(BindValue::U32(params.limit));
    binds.push(BindValue::U32(params.offset));

    Ok(SafeQuery { sql, binds })
}

const RAW_EVENT_COLUMNS: &[&str] = &[
    "id",
    "code",
    "customer_id",
    "tenant_id",
    "timestamp",
    "ingested_at",
    "properties",
    "is_tombstone",
];

const METER_EVENT_COLUMNS: &[&str] = &[
    "id",
    "customer_id",
    "timestamp",
    "ingested_at",
    "properties",
];

/// Conditions on the deduplicated events, so the latest version of an event decides which
/// subscription it belongs to and no two subscriptions read different versions of it.
fn push_attribution(
    attribution: &EventAttribution,
    conditions: &mut Vec<String>,
    binds: &mut Vec<BindValue>,
) {
    if attribution.segments.is_empty() {
        return;
    }
    let segments = attribution
        .segments
        .iter()
        .map(|segment| {
            let mut parts = Vec::new();
            if let Some(from) = segment.from {
                parts.push("timestamp >= toDateTime(?)".to_string());
                binds.push(BindValue::I64(from.timestamp()));
            }
            if let Some(to) = segment.to {
                parts.push("timestamp < toDateTime(?)".to_string());
                binds.push(BindValue::I64(to.timestamp()));
            }
            if let Some(matching) = &segment.matching {
                let path = PropertyColumn(&matching.property).path_sql(binds);
                binds.push(BindValue::String(matching.value.clone()));
                parts.push(format!("{path} = ?"));
            }
            for claim in &segment.excluded {
                let path = PropertyColumn(&claim.property).path_sql(binds);
                binds.push(BindValue::String(claim.value.clone()));
                let mut claimed = vec![
                    format!("{path} = ?"),
                    "timestamp >= toDateTime(?)".to_string(),
                ];
                binds.push(BindValue::I64(claim.from.timestamp()));
                if let Some(to) = claim.to {
                    claimed.push("timestamp < toDateTime(?)".to_string());
                    binds.push(BindValue::I64(to.timestamp()));
                }
                parts.push(format!("NOT ({})", claimed.join(" AND ")));
            }
            if parts.is_empty() {
                "1".to_string()
            } else {
                format!("({})", parts.join(" AND "))
            }
        })
        .collect::<Vec<_>>();
    conditions.push(format!("({})", segments.join(" OR ")));
}

fn push_origin_filter(origin: OriginFilter, _conditions: &mut Vec<String>) {
    match origin {
        OriginFilter::All => {}
    }
}

/// Window bounds are truncated to whole seconds (`toDateTime`): callers that split one window
/// into ranges must truncate their bounds the same way, or events fall in two ranges.
pub fn query_meter_sql(
    params: QueryMeterParams,
    events: &EventsTable,
    database: &str,
) -> Result<SafeQuery, String> {
    let source = events.source(
        database,
        params.tenant_id.as_uuid(),
        EventDedup::new(params.keep_last_events),
    );
    let mut select_binds: Vec<BindValue> = Vec::new();
    let mut subquery_binds: Vec<BindValue> = Vec::new();
    let mut group_by_binds: Vec<BindValue> = Vec::new();
    let mut attribution_binds: Vec<BindValue> = Vec::new();

    // Phase 1: Build subquery conditions
    let mut subquery_conditions = Vec::new();

    subquery_conditions.push("tenant_id = ?".to_string());
    subquery_binds.push(BindValue::Uuid(params.tenant_id.as_uuid()));

    subquery_conditions.push("code = ?".to_string());
    subquery_binds.push(BindValue::String(params.code.clone()));

    subquery_conditions.push("timestamp >= toDateTime(?)".to_string());
    subquery_binds.push(BindValue::I64(params.from.timestamp()));

    // Half-open [from, to): billing periods are half-open and consecutive periods share their
    // boundary instant, so an inclusive upper bound bills the boundary event twice.
    if let Some(to) = params.to {
        subquery_conditions.push("timestamp < toDateTime(?)".to_string());
        subquery_binds.push(BindValue::I64(to.timestamp()));
    }

    let customers: Vec<_> = params.customer_ids.iter().map(|id| id.as_uuid()).collect();
    let mut scope = VoidScope::default()
        .code(&params.code)
        .customers(customers.clone())
        .since(format!("toDateTime({})", params.from.timestamp()));
    if let Some(to) = params.to {
        scope = scope.until(format!("toDateTime({})", to.timestamp()));
    }
    if !customers.is_empty() {
        subquery_conditions.push("customer_id IN ?".to_string());
        subquery_binds.push(BindValue::Uuids(customers));
    }

    if let Some(ref segmentation) = params.segmentation_filter {
        match segmentation {
            SegmentationFilter::Independent(filters) => {
                for (column, values) in filters {
                    if values.is_empty() {
                        return Err(format!("Empty filter for dimension: {column}"));
                    }
                    let col = PropertyColumn(column);
                    let column_condition = values
                        .iter()
                        .map(|value| {
                            let path = col.path_sql(&mut subquery_binds);
                            subquery_binds.push(BindValue::String(value.clone()));
                            format!("{path} = ?")
                        })
                        .collect::<Vec<_>>()
                        .join(" OR ");
                    subquery_conditions.push(format!("({column_condition})"));
                }
            }
            SegmentationFilter::Linked {
                dimension1_key,
                dimension2_key,
                values,
            } => {
                let mut linked_conditions = Vec::new();
                let col1 = PropertyColumn(dimension1_key);
                let col2 = PropertyColumn(dimension2_key);

                for (dim1_value, dim2_values) in values {
                    if dim2_values.is_empty() {
                        let path = col1.path_sql(&mut subquery_binds);
                        subquery_binds.push(BindValue::String(dim1_value.clone()));
                        linked_conditions.push(format!("{path} = ?"));
                    } else {
                        let path1 = col1.path_sql(&mut subquery_binds);
                        subquery_binds.push(BindValue::String(dim1_value.clone()));
                        let path2 = col2.path_sql(&mut subquery_binds);
                        subquery_binds.push(BindValue::Strings(dim2_values.clone()));
                        linked_conditions.push(format!("({path1} = ? AND {path2} IN ?)"));
                    }
                }

                if !linked_conditions.is_empty() {
                    subquery_conditions.push(format!("({})", linked_conditions.join(" OR ")));
                }
            }
        }
    }

    // Pre-aggregation property filters (WHERE-only). Unlike segmentation above, these add
    // NO SELECT/GROUP BY columns — they only restrict which events are aggregated, so the
    // rated result shape is unchanged. An empty `filters` adds nothing (byte-identical SQL).
    push_property_filters(
        &params.filters,
        &mut subquery_conditions,
        &mut subquery_binds,
    );

    push_origin_filter(params.origin, &mut subquery_conditions);

    if let Some(ref value_prop) = params.value_property
        && !matches!(params.aggregation, MeterAggregation::Count)
    {
        let col = PropertyColumn(value_prop);
        let path1 = col.path_sql(&mut subquery_binds);
        subquery_conditions.push(format!("{path1} != ''"));

        // CountDistinct counts distinct raw values (commonly strings), so it skips the guard.
        if !matches!(params.aggregation, MeterAggregation::CountDistinct) {
            subquery_conditions.push(meter_value::numeric_guard(&col, &mut subquery_binds));
        }
    }

    // Phase 2: Build SELECT columns
    let mut select_columns = Vec::new();
    let mut group_by_columns = Vec::new();

    let tz = params
        .window_time_zone
        .unwrap_or(chrono_tz::UTC)
        .name()
        .to_string();

    if let Some(window_size) = &params.window_size {
        let interval = match window_size {
            WindowSize::Minute => "toIntervalMinute(1)",
            WindowSize::Hour => "toIntervalHour(1)",
            WindowSize::Day => "toIntervalDay(1)",
            WindowSize::Week => "toIntervalWeek(1)",
        };

        select_binds.push(BindValue::String(tz.clone()));
        let tumble_start_select =
            format!("tumbleStart(toDateTime(timestamp), {interval}, ?) AS window_start");
        select_binds.push(BindValue::String(tz.clone()));
        let tumble_end_select =
            format!("tumbleEnd(toDateTime(timestamp), {interval}, ?) AS window_end");

        select_columns.push(tumble_start_select);
        select_columns.push(tumble_end_select);
    } else {
        select_columns.push("min(toDateTime(timestamp)) AS window_start".to_string());
        select_columns.push("max(toDateTime(timestamp)) AS window_end".to_string());
    }

    select_columns.extend(meter_value::aggregation_columns(
        &params.aggregation,
        params.value_property.as_deref(),
        &mut select_binds,
    )?);

    // A `customer_id` group-by property is `properties['customer_id']`, not this column.
    let group_by_customer = params.groups_by_customer();
    if group_by_customer {
        select_columns.push("customer_id".to_string());
    }

    for column in &params.group_by {
        let col = PropertyColumn(column);
        select_columns.push(col.select_sql(&mut select_binds));
    }

    let segment_dimensions = params
        .segmentation_filter
        .as_ref()
        .filter(|_| !params.merge_segments);
    if let Some(segmentation) = segment_dimensions {
        match segmentation {
            SegmentationFilter::Independent(filters) => {
                for (column, _) in filters {
                    let col = PropertyColumn(column);
                    select_columns.push(col.select_sql(&mut select_binds));
                }
            }
            SegmentationFilter::Linked {
                dimension1_key,
                dimension2_key,
                ..
            } => {
                let col1 = PropertyColumn(dimension1_key);
                let col2 = PropertyColumn(dimension2_key);
                select_columns.push(col1.select_sql(&mut select_binds));
                select_columns.push(col2.select_sql(&mut select_binds));
            }
        }
    }

    // Phase 3: Build GROUP BY columns
    if let Some(window_size) = &params.window_size {
        let interval = match window_size {
            WindowSize::Minute => "toIntervalMinute(1)",
            WindowSize::Hour => "toIntervalHour(1)",
            WindowSize::Day => "toIntervalDay(1)",
            WindowSize::Week => "toIntervalWeek(1)",
        };
        group_by_binds.push(BindValue::String(tz.clone()));
        group_by_columns.push(format!("tumbleStart(toDateTime(timestamp), {interval}, ?)"));
        group_by_binds.push(BindValue::String(tz.clone()));
        group_by_columns.push(format!("tumbleEnd(toDateTime(timestamp), {interval}, ?)"));
    }

    if group_by_customer {
        group_by_columns.push("customer_id".to_string());
    }

    for column in &params.group_by {
        let col = PropertyColumn(column);
        group_by_columns.push(col.path_sql(&mut group_by_binds));
    }

    if let Some(segmentation) = segment_dimensions {
        match segmentation {
            SegmentationFilter::Independent(filters) => {
                for (column, _) in filters {
                    let col = PropertyColumn(column);
                    group_by_columns.push(col.path_sql(&mut group_by_binds));
                }
            }
            SegmentationFilter::Linked {
                dimension1_key,
                dimension2_key,
                ..
            } => {
                let col1 = PropertyColumn(dimension1_key);
                let col2 = PropertyColumn(dimension2_key);
                group_by_columns.push(col1.path_sql(&mut group_by_binds));
                group_by_columns.push(col2.path_sql(&mut group_by_binds));
            }
        }
    }

    // Phase 4: Live (and keep-last deduped) events
    let from = source.from_sql(
        &METER_EVENT_COLUMNS.join(", "),
        &subquery_conditions.join(" AND "),
        &scope,
    );

    let mut attribution_conditions = Vec::new();
    if let Some(attribution) = &params.attribution {
        push_attribution(
            attribution,
            &mut attribution_conditions,
            &mut attribution_binds,
        );
    }

    // Phase 5: Assemble
    let mut sql = format!("SELECT {} {from}", select_columns.join(", "));
    if !attribution_conditions.is_empty() {
        sql.push_str(&format!(" WHERE {}", attribution_conditions.join(" AND ")));
    }
    if !group_by_columns.is_empty() {
        sql.push_str(&format!(" GROUP BY {}", group_by_columns.join(", ")));
    }
    if params.window_size.is_some() {
        sql.push_str(" ORDER BY window_start");
    }

    let mut binds = select_binds;
    binds.extend(subquery_binds);
    binds.extend(attribution_binds);
    binds.extend(group_by_binds);

    Ok(SafeQuery { sql, binds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        MeterAggregation, PropertyFilter, PropertyFilterOp, QueryMeterParams, SegmentationFilter,
        WindowSize,
    };
    use chrono::{TimeZone, Utc};
    use common_domain::ids::{CustomerId, TenantId};
    use std::collections::HashMap;
    use uuid::Uuid;

    /// Whitespace-insensitive, with the void filter (tested in metric-core) reduced to `LIVE`.
    fn normalize_sql(sql: &str) -> String {
        const LIVE: &str = "AND is_tombstone = 0 AND (";
        let sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let sql = sql.replace("( ", "(").replace(" )", ")");
        let Some(start) = sql.find(LIVE) else {
            return sql;
        };
        let mut depth = 0;
        let mut end = sql.len();
        for (i, c) in sql[start + LIVE.len() - 1..].char_indices() {
            depth += match c {
                '(' => 1,
                ')' => -1,
                _ => 0,
            };
            if depth == 0 {
                end = start + LIVE.len() + i;
                break;
            }
        }
        format!("{}AND LIVE{}", &sql[..start], &sql[end..])
    }

    fn assert_bind_parity(result: &SafeQuery) {
        let placeholders = result.sql.matches('?').count();
        assert_eq!(
            placeholders,
            result.binds.len(),
            "placeholder/bind count mismatch ({} placeholders vs {} binds) for SQL:\n{}",
            placeholders,
            result.binds.len(),
            result.sql
        );
    }

    fn bind_strings(binds: &[BindValue]) -> Vec<String> {
        binds
            .iter()
            .map(|b| match b {
                BindValue::String(s) => format!("S:{s}"),
                BindValue::Strings(v) => format!("A:{}", v.join(",")),
                BindValue::I64(v) => format!("I:{v}"),
                BindValue::U32(v) => format!("U:{v}"),
                BindValue::Uuid(v) => format!("Uuid:{v}"),
                BindValue::Uuids(v) => {
                    format!(
                        "AUuid:{}",
                        v.iter()
                            .map(|u| u.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            })
            .collect()
    }

    fn value_params(aggregation: MeterAggregation) -> QueryMeterParams {
        QueryMeterParams {
            aggregation,
            tenant_id: TenantId::default(),
            code: "c".to_string(),
            value_property: Some("v".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            merge_segments: false,
            attribution: None,
            origin: OriginFilter::All,
            group_by_customer: false,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        }
    }

    #[test]
    fn test_query_meter_min_max_return_the_winning_string() {
        for (aggregation, function) in [
            (MeterAggregation::Min, "argMin"),
            (MeterAggregation::Max, "argMax"),
        ] {
            let result = query_meter_sql(
                value_params(aggregation),
                &"raw_events_v2".into(),
                "meteroid",
            )
            .unwrap();
            let expected = format!(
                r#"
                SELECT
                    min(toDateTime(timestamp)) AS window_start,
                    max(toDateTime(timestamp)) AS window_end,
                    {function}(tuple(properties[?], toString(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))), toFloat64OrNull(properties[?])) AS _winner,
                    _winner.1 AS value_raw,
                    _winner.2 AS value,
                    toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks
                FROM (
                    SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                    WHERE (tenant_id = ?
                        AND code = ?
                        AND timestamp >= toDateTime(?)
                        AND timestamp < toDateTime(?)
                        AND properties[?] != ''
                        AND abs(toFloat64OrNull(properties[?])) < 1e18
                        ) AND is_tombstone = 0 AND (LIVE)
                )
            "#
            );
            assert_eq!(normalize_sql(&result.sql), normalize_sql(&expected));
            assert_bind_parity(&result);
        }
    }

    #[test]
    fn test_query_meter_window_bounds_truncate_to_whole_seconds() {
        let params = QueryMeterParams {
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                + chrono::Duration::milliseconds(999),
            to: Some(
                Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()
                    + chrono::Duration::nanoseconds(1),
            ),
            ..value_params(MeterAggregation::Sum)
        };
        let bs = bind_strings(
            &query_meter_sql(params, &"raw_events_v2".into(), "meteroid")
                .unwrap()
                .binds,
        );
        assert!(bs.contains(&"I:1704067200".to_string()), "{bs:?}");
        assert!(bs.contains(&"I:1704153600".to_string()), "{bs:?}");
    }

    #[test]
    fn test_query_meter_with_minute_window() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test_event".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Minute),
            window_time_zone: Some(chrono_tz::UTC),
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), ?) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), ?)
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:test_event".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
        assert!(bs.contains(&"S:UTC".to_string()));
    }

    #[test]
    fn test_query_meter_with_count() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(count()) AS value
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                    ) AND is_tombstone = 0 AND (LIVE)
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:api_call".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
    }

    #[test]
    fn test_query_meter_with_customer_filter() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("bytes".to_string()),
            customer_ids: vec![
                CustomerId::from(Uuid::from_u128(1)),
                CustomerId::from(Uuid::from_u128(2)),
            ],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Hour),
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalHour(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalHour(1), ?) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                customer_id
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND customer_id IN ?
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalHour(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalHour(1), ?),
                customer_id
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:bytes".to_string()));
        assert!(
            bs.contains(
                &"AUuid:00000000-0000-0000-0000-000000000001,00000000-0000-0000-0000-000000000002"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_query_meter_group_by_customer_without_customer_filter() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: true,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["region".to_string()],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?) AS window_end,
                toString(count()) AS value,
                customer_id,
                properties[?] AS _prop_region
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    ingested_at,
                    properties
                FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?),
                customer_id,
                properties[?]
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let tenant = format!("Uuid:{}", Uuid::max());
        assert_eq!(
            bind_strings(&result.binds),
            vec![
                "S:UTC",
                "S:UTC",
                "S:region",
                tenant.as_str(),
                "S:api_call",
                "I:1704067200",
                "I:1706745600",
                "S:UTC",
                "S:UTC",
                "S:region",
            ]
        );
    }

    #[test]
    fn test_query_meter_customer_id_property_is_grouped_alongside_the_column() {
        // A `customer_id` group-by is the event property, so the column must still be grouped.
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![CustomerId::from(Uuid::from_u128(1))],
            segmentation_filter: None,
            group_by: vec!["customer_id".to_string()],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let sql = normalize_sql(&result.sql);
        assert!(sql.contains(
            "toString(count()) AS value, customer_id, properties[?] AS _prop_customer_id"
        ));
        assert!(sql.ends_with("GROUP BY customer_id, properties[?]"));
        assert_bind_parity(&result);
    }

    #[test]
    fn test_query_meter_with_group_by() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Avg,
            tenant_id: TenantId::default(),
            code: "transaction".to_string(),
            value_property: Some("duration".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["region".to_string(), "endpoint".to_string()],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(count()) AS value_count, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                properties[?] AS _prop_region,
                properties[?] AS _prop_endpoint
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?),
                properties[?],
                properties[?]
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:transaction".to_string()));
        assert!(bs.contains(&"S:duration".to_string()));
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:endpoint".to_string()));
    }

    #[test]
    fn test_query_meter_with_independent_segmentation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "sale".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![
                (
                    "region".to_string(),
                    vec!["us-east".to_string(), "us-west".to_string()],
                ),
                ("tier".to_string(), vec!["premium".to_string()]),
            ])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                properties[?] AS _prop_region,
                properties[?] AS _prop_tier
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND (properties[?] = ? OR properties[?] = ?)
                    AND (properties[?] = ?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY properties[?], properties[?]
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:sale".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:tier".to_string()));
        assert!(bs.contains(&"S:us-east".to_string()));
        assert!(bs.contains(&"S:us-west".to_string()));
        assert!(bs.contains(&"S:premium".to_string()));
    }

    #[test]
    fn test_query_meter_property_filter_in_is_where_only() {
        // An `In` filter adds a WHERE condition but NO SELECT/GROUP BY column (unlike
        // segmentation), so the rated result shape is unchanged.
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            filters: vec![PropertyFilter {
                property: "operation".to_string(),
                op: PropertyFilterOp::In,
                values: vec!["write".to_string(), "delete".to_string()],
            }],
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND (properties[?] = ? OR properties[?] = ?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
        "#;
        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        // No grouping/select column for the filtered property.
        assert!(!result.sql.contains("_prop_operation"));
        assert!(!result.sql.contains("GROUP BY"));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:operation".to_string()));
        assert!(bs.contains(&"S:write".to_string()));
        assert!(bs.contains(&"S:delete".to_string()));
    }

    #[test]
    fn test_query_meter_property_filter_not_in_requires_presence() {
        // `NotIn` is presence-required: mapContains guards against an absent key passing `!=`.
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            filters: vec![PropertyFilter {
                property: "tier".to_string(),
                op: PropertyFilterOp::NotIn,
                values: vec!["free".to_string()],
            }],
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(count()) AS value
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND (mapContains(properties, ?) AND properties[?] != ?)
                    ) AND is_tombstone = 0 AND (LIVE)
            )
        "#;
        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:tier".to_string()));
        assert!(bs.contains(&"S:free".to_string()));
    }

    fn attributed(attribution: Option<EventAttribution>) -> QueryMeterParams {
        QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "tokens".to_string(),
            value_property: Some("tokens".to_string()),
            customer_ids: vec![CustomerId::default()],
            segmentation_filter: None,
            filters: vec![],
            group_by: vec!["model".to_string()],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()),
            keep_last_events: false,
        }
    }

    #[test]
    fn test_query_meter_attribution_filters_the_deduplicated_events() {
        use crate::domain::{AttributionSegment, EventClaim, EventMatch};
        let change = Utc.with_ymd_and_hms(2026, 9, 10, 0, 0, 0).unwrap();
        let claim_to = Utc.with_ymd_and_hms(2026, 9, 20, 14, 30, 0).unwrap();
        let result = query_meter_sql(
            attributed(Some(EventAttribution {
                segments: vec![
                    AttributionSegment {
                        from: None,
                        to: Some(change),
                        matching: Some(EventMatch {
                            property: "project".to_string(),
                            value: "p1".to_string(),
                        }),
                        excluded: vec![],
                    },
                    AttributionSegment {
                        from: Some(change),
                        to: None,
                        matching: None,
                        excluded: vec![EventClaim {
                            property: "api_key".to_string(),
                            value: "key-A".to_string(),
                            from: change,
                            to: Some(claim_to),
                        }],
                    },
                ],
            })),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                customer_id,
                properties[?] AS _prop_model
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties
                FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                    AND customer_id IN ?
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            WHERE ((timestamp < toDateTime(?) AND properties[?] = ?)
                OR (timestamp >= toDateTime(?)
                    AND NOT (properties[?] = ? AND timestamp >= toDateTime(?) AND timestamp < toDateTime(?))))
            GROUP BY customer_id, properties[?]
        "#;
        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let binds = bind_strings(&result.binds);
        let attribution_binds = &binds[binds.len() - 9..binds.len() - 1];
        assert_eq!(
            attribution_binds,
            [
                format!("I:{}", change.timestamp()),
                "S:project".to_string(),
                "S:p1".to_string(),
                format!("I:{}", change.timestamp()),
                "S:api_key".to_string(),
                "S:key-A".to_string(),
                format!("I:{}", change.timestamp()),
                format!("I:{}", claim_to.timestamp()),
            ]
        );
        assert_eq!(binds.last(), Some(&"S:model".to_string()));
    }

    #[test]
    fn test_keep_last_tenant_attributes_the_last_ingested_copy() {
        use crate::domain::{AttributionSegment, EventMatch};
        let mut params = attributed(Some(EventAttribution {
            segments: vec![AttributionSegment {
                from: None,
                to: None,
                matching: Some(EventMatch {
                    property: "project".to_string(),
                    value: "p1".to_string(),
                }),
                excluded: vec![],
            }],
        }));
        params.keep_last_events = true;
        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        assert!(
            normalize_sql(&result.sql).contains(
                "ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id) WHERE ((properties[?] = ?))"
            ),
            "{}",
            result.sql
        );
        assert_bind_parity(&result);
    }

    #[test]
    fn test_query_meter_without_attribution_is_unchanged() {
        let none = query_meter_sql(attributed(None), &"raw_events_v2".into(), "meteroid").unwrap();
        let empty = query_meter_sql(
            attributed(Some(EventAttribution::default())),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        assert_eq!(none.sql, empty.sql);
        assert_eq!(bind_strings(&none.binds), bind_strings(&empty.binds));
        assert!(!none.sql.contains(") WHERE"));
    }

    #[test]
    fn test_query_meter_empty_property_filters_is_noop() {
        // The safety invariant: no filters ⇒ identical SQL to the pre-feature baseline.
        let base = |filters: Vec<PropertyFilter>| QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            filters,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };
        let empty = query_meter_sql(base(vec![]), &"raw_events_v2".into(), "meteroid").unwrap();
        // A filter with no values is skipped, so it must produce the same SQL as no filter.
        let empty_values = query_meter_sql(
            base(vec![PropertyFilter {
                property: "x".to_string(),
                op: PropertyFilterOp::In,
                values: vec![],
            }]),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        assert_eq!(normalize_sql(&empty.sql), normalize_sql(&empty_values.sql));
    }

    #[test]
    fn test_query_meter_with_linked_segmentation() {
        let mut values = HashMap::new();
        values.insert(
            "prod1".to_string(),
            vec!["v1".to_string(), "v2".to_string()],
        );
        values.insert("prod2".to_string(), vec!["v3".to_string()]);

        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("count".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Linked {
                dimension1_key: "product".to_string(),
                dimension2_key: "version".to_string(),
                values,
            }),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();

        // HashMap iteration order is not guaranteed, so check both possible orders
        let expected1 = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                properties[?] AS _prop_product,
                properties[?] AS _prop_version
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND ((properties[?] = ? AND properties[?] IN ?)
                         OR (properties[?] = ? AND properties[?] IN ?))
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY properties[?], properties[?]
        "#;

        let expected2 = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                properties[?] AS _prop_product,
                properties[?] AS _prop_version
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND ((properties[?] = ? AND properties[?] IN ?)
                         OR (properties[?] = ? AND properties[?] IN ?))
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY properties[?], properties[?]
        "#;

        let result_normalized = normalize_sql(&result.sql);
        assert!(
            result_normalized == normalize_sql(expected1)
                || result_normalized == normalize_sql(expected2),
            "SQL did not match either expected variant.\nGot:\n{}\n\nExpected either:\n{}\n\nor:\n{}",
            result.sql,
            expected1,
            expected2
        );
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:count".to_string()));
        assert!(bs.contains(&"S:product".to_string()));
        assert!(bs.contains(&"S:version".to_string()));
        assert!(bs.contains(&"S:prod1".to_string()));
        assert!(bs.contains(&"S:prod2".to_string()));
    }

    #[test]
    fn test_query_meter_merged_segments_filter_without_breakdown() {
        let customer = CustomerId::from(Uuid::from_u128(1));
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: true,
            aggregation: MeterAggregation::Max,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("count".to_string()),
            customer_ids: vec![customer],
            segmentation_filter: Some(SegmentationFilter::Linked {
                dimension1_key: "product".to_string(),
                dimension2_key: "version".to_string(),
                values: HashMap::from([("prod1".to_string(), vec!["v1".to_string()])]),
            }),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                argMax(tuple(properties[?], toString(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))), toFloat64OrNull(properties[?])) AS _winner, _winner.1 AS value_raw, _winner.2 AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                customer_id
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    ingested_at,
                    properties
                FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND customer_id IN ?
                    AND ((properties[?] = ? AND properties[?] IN ?))
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY customer_id
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:prod1".to_string()));
        assert!(bs.contains(&"S:product".to_string()));
    }

    #[test]
    fn test_query_meter_with_count_distinct() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::CountDistinct,
            tenant_id: TenantId::default(),
            code: "login".to_string(),
            value_property: Some("user_id".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?) AS window_end,
                toString(uniqExact(properties[?])) AS value
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?)
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:login".to_string()));
        assert!(bs.contains(&"S:user_id".to_string()));
    }

    #[test]
    fn test_query_meter_with_latest_aggregation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Latest,
            tenant_id: TenantId::default(),
            code: "status".to_string(),
            value_property: Some("value".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                argMax(tuple(properties[?], toString(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))), (timestamp, ingested_at, id)) AS _winner, _winner.1 AS value_raw, _winner.2 AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:status".to_string()));
        assert!(bs.contains(&"S:value".to_string()));
    }

    #[test]
    fn test_query_meter_validates_empty_segmentation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![(
                "region".to_string(),
                vec![],
            )])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Empty filter for dimension: region");
    }

    #[test]
    fn test_query_meter_validates_value_property_required() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "value_property is required for non-Count aggregations"
        );
    }

    #[test]
    fn test_query_meter_includes_value_property_filter() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "purchase".to_string(),
            value_property: Some("price".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:purchase".to_string()));
        assert!(bs.contains(&"S:price".to_string()));
    }

    #[test]
    fn test_query_meter_time_filters_in_cte() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(count()) AS value
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                    ) AND is_tombstone = 0 AND (LIVE)
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:event".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
    }

    #[test]
    fn test_query_meter_upper_bound_is_exclusive() {
        // Consecutive billing periods share their boundary instant (arrear end == next advance
        // start), so an inclusive upper bound would rate the boundary event on both invoices.
        let boundary = Utc.with_ymd_and_hms(2026, 7, 31, 0, 0, 0).unwrap();
        let params = |from, to| QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "memory".to_string(),
            value_property: Some("gib".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from,
            to: Some(to),
            keep_last_events: false,
        };

        let first = query_meter_sql(
            params(
                Utc.with_ymd_and_hms(2026, 7, 16, 0, 0, 0).unwrap(),
                boundary,
            ),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();
        let second = query_meter_sql(
            params(
                boundary,
                Utc.with_ymd_and_hms(2026, 8, 31, 0, 0, 0).unwrap(),
            ),
            &"raw_events_v2".into(),
            "meteroid",
        )
        .unwrap();

        assert!(first.sql.contains("timestamp < toDateTime(?)"));
        assert!(!first.sql.contains("timestamp <= toDateTime(?)"));
        assert!(second.sql.contains("timestamp >= toDateTime(?)"));
    }

    #[test]
    fn test_query_meter_with_reserved_column_in_group_by() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "purchase".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["code".to_string()],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                properties[?] AS _prop_code
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY properties[?]
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:purchase".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"S:code".to_string()));
    }

    #[test]
    fn test_query_meter_with_customer_id_in_segmentation() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "usage".to_string(),
            value_property: Some("bytes".to_string()),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![(
                "customer_id".to_string(),
                vec!["cust1".to_string(), "cust2".to_string()],
            )])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(sum(ifNull(toDecimal128OrNull(properties[?], 9), accurateCastOrNull(toFloat64OrNull(properties[?]), 'Decimal(38, 9)')))) AS value, toString(countIf(isNull(toDecimal128OrNull(properties[?], 9)))) AS decimal_fallbacks,
                properties[?] AS _prop_customer_id
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND (properties[?] = ? OR properties[?] = ?)
                    AND properties[?] != ''
                    AND abs(toFloat64OrNull(properties[?])) < 1e18
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY properties[?]
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:bytes".to_string()));
        assert!(bs.contains(&"S:customer_id".to_string()));
        assert!(bs.contains(&"S:cust1".to_string()));
        assert!(bs.contains(&"S:cust2".to_string()));
    }

    #[test]
    fn test_query_meter_mixed_reserved_and_custom_columns() {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![
                "customer_id".to_string(),
                "region".to_string(),
                "code".to_string(),
            ],
            window_size: None,
            window_time_zone: None,
            filters: vec![],

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            keep_last_events: false,
        };

        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toString(count()) AS value,
                properties[?] AS _prop_customer_id,
                properties[?] AS _prop_region,
                properties[?] AS _prop_code
            FROM (
                SELECT id, customer_id, timestamp, ingested_at, properties FROM raw_events_v2
                WHERE (tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    ) AND is_tombstone = 0 AND (LIVE)
            )
            GROUP BY properties[?], properties[?], properties[?]
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:event".to_string()));
        assert!(bs.contains(&"S:customer_id".to_string()));
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:code".to_string()));
    }

    #[test]
    fn test_query_raw_events_basic() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampDesc,
            filters: vec![],
            after: None,
            keep_last_events: false,
        };

        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone
            FROM raw_events_v2
            WHERE (tenant_id = ?
                AND timestamp >= toDateTime(?)
                AND timestamp < toDateTime(?)
                ) AND is_tombstone = 0 AND (LIVE)
            ORDER BY timestamp DESC, id DESC LIMIT ? OFFSET ?
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"Uuid:ffffffff-ffff-ffff-ffff-ffffffffffff".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
        assert!(bs.contains(&"U:10".to_string()));
        assert!(bs.contains(&"U:0".to_string()));
    }

    #[test]
    fn test_query_raw_events_with_customer_and_code_filters() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 20,
            offset: 5,
            search: None,
            event_codes: vec!["api_call".to_string(), "storage".to_string()],
            customer_ids: vec![
                CustomerId::from(Uuid::from_u128(1)),
                CustomerId::from(Uuid::from_u128(2)),
            ],
            sort_order: EventSortOrder::IngestedDesc,
            filters: vec![],
            after: None,
            keep_last_events: false,
        };

        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone
            FROM raw_events_v2
            WHERE (tenant_id = ?
                AND timestamp >= toDateTime(?)
                AND timestamp < toDateTime(?)
                AND customer_id IN ?
                AND code IN ?
                ) AND is_tombstone = 0 AND (LIVE)
            ORDER BY ingested_at DESC LIMIT ? OFFSET ?
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let voids = "FROM meteroid.event_voids_local WHERE tenant_id = \
            toUUID('ffffffff-ffff-ffff-ffff-ffffffffffff') AND code IN ('api_call', 'storage') AND \
            customer_id IN (toUUID('00000000-0000-0000-0000-000000000001'), \
            toUUID('00000000-0000-0000-0000-000000000002')) AND timestamp >= \
            toDateTime(1704067200) AND timestamp <= toDateTime(1704153600)";
        assert_eq!(result.sql.matches(voids).count(), 2, "{}", result.sql);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"A:api_call,storage".to_string()));
        assert!(bs.contains(&"U:20".to_string()));
        assert!(bs.contains(&"U:5".to_string()));
    }

    #[test]
    fn test_query_raw_events_with_property_filters() {
        // The events explorer applies the metric's filters (IN + presence-required NOT_IN)
        // to the events it lists, so it lists only the events the metric would count.
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            filters: vec![
                PropertyFilter {
                    property: "region".to_string(),
                    op: PropertyFilterOp::In,
                    values: vec!["eu-west".to_string(), "us-east".to_string()],
                },
                PropertyFilter {
                    property: "tier".to_string(),
                    op: PropertyFilterOp::NotIn,
                    values: vec!["free".to_string()],
                },
            ],
            sort_order: EventSortOrder::TimestampDesc,
            after: None,
            keep_last_events: false,
        };

        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let n = normalize_sql(&result.sql);
        assert!(
            n.contains("(properties[?] = ? OR properties[?] = ?)"),
            "{n}"
        );
        assert!(
            n.contains("(mapContains(properties, ?) AND properties[?] != ?)"),
            "{n}"
        );
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:region".to_string()));
        assert!(bs.contains(&"S:eu-west".to_string()));
        assert!(bs.contains(&"S:tier".to_string()));
        assert!(bs.contains(&"S:free".to_string()));
    }

    #[test]
    fn test_query_raw_events_with_search() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: Some("foo".to_string()),
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampAsc,
            filters: vec![],
            after: None,
            keep_last_events: false,
        };

        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone
            FROM raw_events_v2
            WHERE (tenant_id = ?
                AND timestamp >= toDateTime(?)
                AND timestamp < toDateTime(?)
                AND (id ILIKE ? OR code ILIKE ? OR arrayStringConcat(mapValues(properties), ' ') ILIKE ?)
                ) AND is_tombstone = 0 AND (LIVE)
            ORDER BY timestamp ASC, id ASC LIMIT ? OFFSET ?
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert_eq!(bs.iter().filter(|b| *b == "S:%foo%").count(), 3);
    }

    #[test]
    fn test_query_raw_events_keyset() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampAsc,
            filters: vec![],
            after: Some(crate::domain::EventCursor {
                timestamp_nanos: 1_704_067_200_000_000_001,
                id: "evt_1".to_string(),
            }),
            keep_last_events: false,
        };

        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone
            FROM raw_events_v2
            WHERE (tenant_id = ?
                AND timestamp >= toDateTime(?)
                AND timestamp < toDateTime(?)
                ) AND is_tombstone = 0 AND (LIVE)
                AND (toUnixTimestamp64Nano(timestamp) > ? OR (toUnixTimestamp64Nano(timestamp) = ? AND id > ?))
            ORDER BY timestamp ASC, id ASC LIMIT ? OFFSET ?
        "#;
        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        assert_bind_parity(&result);
        let bs = bind_strings(&result.binds);
        assert_eq!(
            bs.iter().filter(|b| *b == "I:1704067200000000001").count(),
            2
        );
        assert!(bs.contains(&"S:evt_1".to_string()));
    }

    #[test]
    fn test_query_raw_events_keyset_rejects_ingested_order() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::IngestedAsc,
            filters: vec![],
            after: Some(crate::domain::EventCursor {
                timestamp_nanos: 0,
                id: String::new(),
            }),
            keep_last_events: false,
        };
        assert!(query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").is_err());
    }

    fn count_meter_sql(keep_last_events: bool) -> SafeQuery {
        let params = QueryMeterParams {
            origin: OriginFilter::All,
            group_by_customer: false,
            attribution: None,
            merge_segments: false,
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            filters: vec![],
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            keep_last_events,
        };
        let result = query_meter_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        assert_bind_parity(&result);
        result
    }

    const COUNT_METER_SUBQUERY: &str = "SELECT id, customer_id, timestamp, ingested_at, properties \
        FROM raw_events_v2 \
        WHERE (tenant_id = ? AND code = ? \
        AND timestamp >= toDateTime(?) AND timestamp < toDateTime(?) \
        ) AND is_tombstone = 0 AND (LIVE)";

    #[test]
    fn test_query_meter_keep_last_tenant_keeps_last_ingested() {
        let result = count_meter_sql(true);
        let expected = format!(
            "SELECT min(toDateTime(timestamp)) AS window_start, \
             max(toDateTime(timestamp)) AS window_end, toString(count()) AS value \
             FROM ( {COUNT_METER_SUBQUERY} \
             ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id )"
        );
        assert_eq!(normalize_sql(&result.sql), normalize_sql(&expected));
    }

    #[test]
    fn test_query_meter_default_tenant_has_no_sort() {
        let with = count_meter_sql(true);
        let without = count_meter_sql(false);
        let expected = format!(
            "SELECT min(toDateTime(timestamp)) AS window_start, \
             max(toDateTime(timestamp)) AS window_end, toString(count()) AS value \
             FROM ( {COUNT_METER_SUBQUERY} )"
        );
        assert_eq!(normalize_sql(&without.sql), normalize_sql(&expected));
        assert_eq!(bind_strings(&with.binds), bind_strings(&without.binds));
    }

    fn raw_events_sql(keep_last_events: bool) -> String {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 10,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::IngestedDesc,
            filters: vec![],
            after: None,
            keep_last_events,
        };
        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        assert_bind_parity(&result);
        normalize_sql(&result.sql)
    }

    const RAW_EVENTS_SUBQUERY: &str = "SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone \
        FROM raw_events_v2 \
        WHERE (tenant_id = ? AND timestamp >= toDateTime(?) AND timestamp < toDateTime(?) \
        ) AND is_tombstone = 0 AND (LIVE)";

    #[test]
    fn test_query_raw_events_keep_last_tenant_keeps_last_ingested() {
        let expected = format!(
            "SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone \
             FROM ( {RAW_EVENTS_SUBQUERY} ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id ) \
             ORDER BY ingested_at DESC LIMIT ? OFFSET ?"
        );
        assert_eq!(raw_events_sql(true), normalize_sql(&expected));
    }

    #[test]
    fn test_query_raw_events_default_tenant_reads_the_table_flat() {
        let expected = format!("{RAW_EVENTS_SUBQUERY} ORDER BY ingested_at DESC LIMIT ? OFFSET ?");
        assert_eq!(raw_events_sql(false), normalize_sql(&expected));
    }

    #[test]
    fn test_query_raw_events_keep_last_keyset_filters_the_deduplicated_events() {
        let params = QueryRawEventsParams {
            tenant_id: TenantId::default(),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            limit: 1,
            offset: 0,
            search: None,
            event_codes: vec![],
            customer_ids: vec![],
            sort_order: EventSortOrder::TimestampAsc,
            filters: vec![],
            after: Some(crate::domain::EventCursor {
                timestamp_nanos: 7,
                id: "evt_1".to_string(),
            }),
            keep_last_events: true,
        };
        let result = query_raw_events_sql(params, &"raw_events_v2".into(), "meteroid").unwrap();
        let expected = format!(
            "SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties, is_tombstone \
             FROM ( {RAW_EVENTS_SUBQUERY} ORDER BY ingested_at DESC LIMIT 1 BY id, customer_id ) \
             WHERE (toUnixTimestamp64Nano(timestamp) > ? OR (toUnixTimestamp64Nano(timestamp) = ? AND id > ?)) \
             ORDER BY timestamp ASC, id ASC LIMIT ? OFFSET ?"
        );
        assert_eq!(normalize_sql(&result.sql), normalize_sql(&expected));
        assert_bind_parity(&result);
        assert_eq!(
            bind_strings(&result.binds)[3..],
            ["I:7", "I:7", "S:evt_1", "U:1", "U:0"]
        );
    }
}
