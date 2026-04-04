use crate::connectors::clickhouse::sql::{BindValue, PropertyColumn, SafeQuery};
use crate::domain::{
    EventSortOrder, MeterAggregation, QueryMeterParams, QueryRawEventsParams, SegmentationFilter,
    WindowSize,
};
use chrono::Utc;
use common_domain::ids::BaseId;

pub fn query_raw_events_sql(
    params: QueryRawEventsParams,
    events_table: &str,
) -> Result<SafeQuery, String> {
    let mut conditions = Vec::new();
    let mut binds: Vec<BindValue> = Vec::new();

    conditions.push("tenant_id = ?".to_string());
    binds.push(BindValue::Uuid(params.tenant_id.as_uuid()));

    conditions.push("timestamp >= toDateTime(?)".to_string());
    binds.push(BindValue::I64(params.from.timestamp()));

    if let Some(to) = params.to {
        conditions.push("timestamp < toDateTime(?)".to_string());
        binds.push(BindValue::I64(to.timestamp()));
    } else {
        conditions.push("timestamp < toDateTime(?)".to_string());
        binds.push(BindValue::I64(Utc::now().timestamp()));
    }

    if !params.customer_ids.is_empty() {
        conditions.push("customer_id IN ?".to_string());
        binds.push(BindValue::Uuids(
            params.customer_ids.iter().map(|id| id.as_uuid()).collect(),
        ));
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

    let order_by = match params.sort_order {
        EventSortOrder::TimestampDesc => "timestamp DESC",
        EventSortOrder::TimestampAsc => "timestamp ASC",
        EventSortOrder::IngestedDesc => "ingested_at DESC",
        EventSortOrder::IngestedAsc => "ingested_at ASC",
    };

    let dedup_subquery = build_dedup_subquery(RAW_EVENT_COLUMNS, events_table, &conditions);

    let columns = RAW_EVENT_COLUMNS.join(", ");
    let sql =
        format!("SELECT {columns} FROM ( {dedup_subquery} ) ORDER BY {order_by} LIMIT ? OFFSET ?");
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
];

const METER_EVENT_COLUMNS: &[&str] = &["id", "customer_id", "timestamp", "properties"];

fn build_dedup_subquery(columns: &[&str], events_table: &str, conditions: &[String]) -> String {
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    let columns = columns.join(", ");
    format!(
        "SELECT {columns} FROM {events_table} {where_clause} ORDER BY timestamp DESC LIMIT 1 BY id, customer_id"
    )
}

pub fn query_meter_sql(params: QueryMeterParams, events_table: &str) -> Result<SafeQuery, String> {
    let mut select_binds: Vec<BindValue> = Vec::new();
    let mut subquery_binds: Vec<BindValue> = Vec::new();
    let mut group_by_binds: Vec<BindValue> = Vec::new();

    // Phase 1: Build subquery conditions
    let mut subquery_conditions = Vec::new();

    subquery_conditions.push("tenant_id = ?".to_string());
    subquery_binds.push(BindValue::Uuid(params.tenant_id.as_uuid()));

    subquery_conditions.push("code = ?".to_string());
    subquery_binds.push(BindValue::String(params.code.clone()));

    subquery_conditions.push("timestamp >= toDateTime(?)".to_string());
    subquery_binds.push(BindValue::I64(params.from.timestamp()));

    if let Some(to) = params.to {
        subquery_conditions.push("timestamp <= toDateTime(?)".to_string());
        subquery_binds.push(BindValue::I64(to.timestamp()));
    }

    if !params.customer_ids.is_empty() {
        subquery_conditions.push("customer_id IN ?".to_string());
        subquery_binds.push(BindValue::Uuids(
            params.customer_ids.iter().map(|id| id.as_uuid()).collect(),
        ));
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

    if let Some(ref value_prop) = params.value_property
        && !matches!(params.aggregation, MeterAggregation::Count)
    {
        let col = PropertyColumn(value_prop);
        let path1 = col.path_sql(&mut subquery_binds);
        let path2 = col.path_sql(&mut subquery_binds);
        subquery_conditions.push(format!(
            "{path1} != '' AND isNotNull(toFloat64OrNull({path2}))"
        ));
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

    // Value expression + aggregation
    let value_expr = if let Some(ref value_prop) = params.value_property {
        let col = PropertyColumn(value_prop);
        let path = col.path_sql(&mut select_binds);
        format!("toFloat64OrZero({path})")
    } else if matches!(params.aggregation, MeterAggregation::Count) {
        "1".to_string()
    } else {
        return Err("value_property is required for non-Count aggregations".to_string());
    };

    let aggregation_column = match &params.aggregation {
        MeterAggregation::Sum => format!("sum({value_expr}) AS value"),
        MeterAggregation::Avg => format!("avg({value_expr}) AS value"),
        MeterAggregation::Min => format!("min({value_expr}) AS value"),
        MeterAggregation::Max => format!("max({value_expr}) AS value"),
        MeterAggregation::Count => "toFloat64(count(*)) AS value".to_string(),
        MeterAggregation::Latest => {
            format!("argMax({value_expr}, toDateTime(timestamp)) AS value")
        }
        MeterAggregation::CountDistinct => {
            if let Some(ref value_prop) = params.value_property {
                let col = PropertyColumn(value_prop);
                let path = col.path_sql(&mut select_binds);
                format!("toFloat64(uniq({path})) AS value")
            } else {
                return Err("value_property is required for CountDistinct aggregation".to_string());
            }
        }
    };
    select_columns.push(aggregation_column);

    let add_customer_id_group_by =
        !params.customer_ids.is_empty() && !params.group_by.contains(&"customer_id".to_string());
    if !params.customer_ids.is_empty() {
        select_columns.push("customer_id".to_string());
    }

    for column in &params.group_by {
        let col = PropertyColumn(column);
        select_columns.push(col.select_sql(&mut select_binds));
    }

    if let Some(ref segmentation) = params.segmentation_filter {
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
        };
        group_by_binds.push(BindValue::String(tz.clone()));
        group_by_columns.push(format!("tumbleStart(toDateTime(timestamp), {interval}, ?)"));
        group_by_binds.push(BindValue::String(tz.clone()));
        group_by_columns.push(format!("tumbleEnd(toDateTime(timestamp), {interval}, ?)"));
    }

    if add_customer_id_group_by {
        group_by_columns.push("customer_id".to_string());
    }

    for column in &params.group_by {
        let col = PropertyColumn(column);
        group_by_columns.push(col.path_sql(&mut group_by_binds));
    }

    if let Some(ref segmentation) = params.segmentation_filter {
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

    // Phase 4: Deduplication subquery
    let dedup_subquery =
        build_dedup_subquery(METER_EVENT_COLUMNS, events_table, &subquery_conditions);

    // Phase 5: Assemble
    let mut sql = format!("SELECT {}", select_columns.join(", "));
    sql.push_str(&format!(" FROM ( {dedup_subquery} )"));
    if !group_by_columns.is_empty() {
        sql.push_str(&format!(" GROUP BY {}", group_by_columns.join(", ")));
    }
    if params.window_size.is_some() {
        sql.push_str(" ORDER BY window_start");
    }

    let mut binds = select_binds;
    binds.extend(subquery_binds);
    binds.extend(group_by_binds);

    Ok(SafeQuery { sql, binds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MeterAggregation, QueryMeterParams, SegmentationFilter, WindowSize};
    use chrono::{TimeZone, Utc};
    use common_domain::ids::{CustomerId, TenantId};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn normalize_sql(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
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

    #[test]
    fn test_query_meter_dedup_with_minute_window() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test_event".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Minute),
            window_time_zone: Some(chrono_tz::UTC),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), ?) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp <= toDateTime(?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), ?)
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:test_event".to_string()));
        assert!(bs.contains(&"S:amount".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
        assert!(bs.contains(&"S:UTC".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_with_count() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "api_call".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toFloat64(count(*)) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp <= toDateTime(?)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:api_call".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_with_customer_filter() {
        let params = QueryMeterParams {
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

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalHour(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalHour(1), ?) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value,
                customer_id
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND customer_id IN ?
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
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
    fn test_query_meter_dedup_with_group_by() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Avg,
            tenant_id: TenantId::default(),
            code: "transaction".to_string(),
            value_property: Some("duration".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["region".to_string(), "endpoint".to_string()],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?) AS window_end,
                avg(toFloat64OrZero(properties[?])) AS value,
                properties[?] AS _prop_region,
                properties[?] AS _prop_endpoint
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
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
    fn test_query_meter_dedup_with_independent_segmentation() {
        let params = QueryMeterParams {
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

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value,
                properties[?] AS _prop_region,
                properties[?] AS _prop_tier
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND (properties[?] = ? OR properties[?] = ?)
                    AND (properties[?] = ?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            GROUP BY properties[?], properties[?]
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
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
    fn test_query_meter_dedup_with_linked_segmentation() {
        let mut values = HashMap::new();
        values.insert(
            "prod1".to_string(),
            vec!["v1".to_string(), "v2".to_string()],
        );
        values.insert("prod2".to_string(), vec!["v3".to_string()]);

        let params = QueryMeterParams {
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

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();

        // HashMap iteration order is not guaranteed, so check both possible orders
        let expected1 = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value,
                properties[?] AS _prop_product,
                properties[?] AS _prop_version
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND ((properties[?] = ? AND properties[?] IN ?)
                         OR (properties[?] = ? AND properties[?] IN ?))
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            GROUP BY properties[?], properties[?]
        "#;

        let expected2 = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value,
                properties[?] AS _prop_product,
                properties[?] AS _prop_version
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND ((properties[?] = ? AND properties[?] IN ?)
                         OR (properties[?] = ? AND properties[?] IN ?))
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
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
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:usage".to_string()));
        assert!(bs.contains(&"S:count".to_string()));
        assert!(bs.contains(&"S:product".to_string()));
        assert!(bs.contains(&"S:version".to_string()));
        assert!(bs.contains(&"S:prod1".to_string()));
        assert!(bs.contains(&"S:prod2".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_with_count_distinct() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::CountDistinct,
            tenant_id: TenantId::default(),
            code: "login".to_string(),
            value_property: Some("user_id".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?) AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?) AS window_end,
                toFloat64(uniq(properties[?])) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), ?),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), ?)
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:login".to_string()));
        assert!(bs.contains(&"S:user_id".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_with_latest_aggregation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Latest,
            tenant_id: TenantId::default(),
            code: "status".to_string(),
            value_property: Some("value".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                argMax(toFloat64OrZero(properties[?]), toDateTime(timestamp)) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:status".to_string()));
        assert!(bs.contains(&"S:value".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_validates_empty_segmentation() {
        let params = QueryMeterParams {
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

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Empty filter for dimension: region");
    }

    #[test]
    fn test_query_meter_dedup_validates_value_property_required() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "test".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "value_property is required for non-Count aggregations"
        );
    }

    #[test]
    fn test_query_meter_dedup_includes_value_property_filter() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "purchase".to_string(),
            value_property: Some("price".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:purchase".to_string()));
        assert!(bs.contains(&"S:price".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_time_filters_in_cte() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Count,
            tenant_id: TenantId::default(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toFloat64(count(*)) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp <= toDateTime(?)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"S:event".to_string()));
        assert!(bs.contains(&"I:1704067200".to_string()));
        assert!(bs.contains(&"I:1704153600".to_string()));
    }

    #[test]
    fn test_query_meter_dedup_with_reserved_column_in_group_by() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            tenant_id: TenantId::default(),
            code: "purchase".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["code".to_string()],
            window_size: None,
            window_time_zone: None,

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value,
                properties[?] AS _prop_code
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
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
    fn test_query_meter_dedup_with_customer_id_in_segmentation() {
        let params = QueryMeterParams {
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

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties[?])) AS value,
                properties[?] AS _prop_customer_id
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                    AND (properties[?] = ? OR properties[?] = ?)
                    AND properties[?] != ''
                    AND isNotNull(toFloat64OrNull(properties[?]))
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
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
    fn test_query_meter_dedup_mixed_reserved_and_custom_columns() {
        let params = QueryMeterParams {
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

            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toFloat64(count(*)) AS value,
                properties[?] AS _prop_customer_id,
                properties[?] AS _prop_region,
                properties[?] AS _prop_code
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND code = ?
                    AND timestamp >= toDateTime(?)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
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
        };

        let result = query_raw_events_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties
            FROM (
                SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                ORDER BY timestamp DESC LIMIT 1 BY id, customer_id
            )
            ORDER BY timestamp DESC LIMIT ? OFFSET ?
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
        };

        let result = query_raw_events_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties
            FROM (
                SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                    AND customer_id IN ?
                    AND code IN ?
                ORDER BY timestamp DESC LIMIT 1 BY id, customer_id
            )
            ORDER BY ingested_at DESC LIMIT ? OFFSET ?
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert!(bs.contains(&"A:api_call,storage".to_string()));
        assert!(bs.contains(&"U:20".to_string()));
        assert!(bs.contains(&"U:5".to_string()));
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
        };

        let result = query_raw_events_sql(params, "raw_events_v2").unwrap();
        let expected = r#"
            SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties
            FROM (
                SELECT id, code, customer_id, tenant_id, timestamp, ingested_at, properties
                FROM raw_events_v2
                WHERE tenant_id = ?
                    AND timestamp >= toDateTime(?)
                    AND timestamp < toDateTime(?)
                    AND (id ILIKE ? OR code ILIKE ? OR arrayStringConcat(mapValues(properties), ' ') ILIKE ?)
                ORDER BY timestamp DESC LIMIT 1 BY id, customer_id
            )
            ORDER BY timestamp ASC LIMIT ? OFFSET ?
        "#;

        assert_eq!(normalize_sql(&result.sql), normalize_sql(expected));
        let bs = bind_strings(&result.binds);
        assert_eq!(bs.iter().filter(|b| *b == "S:%foo%").count(), 3);
    }
}
