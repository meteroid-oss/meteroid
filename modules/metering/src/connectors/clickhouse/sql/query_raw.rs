use crate::connectors::clickhouse::sql::init::get_events_table_name;
use crate::connectors::clickhouse::sql::{PropertyColumn, escape_sql_identifier};
use crate::domain::{
    EventSortOrder, MeterAggregation, QueryMeterParams, QueryRawEventsParams, SegmentationFilter,
    WindowSize,
};
use chrono::{DateTime, Utc};

// TODO improve
pub fn query_raw_event_table_sql(
    tenant_id: String,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: i32,
) -> String {
    let table_name = get_events_table_name();
    let mut where_clauses = Vec::new();

    let mut query = format!(
        "SELECT id, code, customer_id, timestamp, ingested_at, properties FROM {table_name}"
    );

    where_clauses.push(format!("tenant_id = '{tenant_id}'"));

    if let Some(from_time) = from {
        where_clauses.push(format!("timestamp >= {}", from_time.timestamp()));
    }

    if let Some(to_time) = to {
        where_clauses.push(format!("timestamp <= {}", to_time.timestamp()));
    }

    if !where_clauses.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&where_clauses.join(" AND "));
    }

    query.push_str(&format!(" ORDER BY time DESC LIMIT {limit}"));

    query
}

pub fn query_raw_events_sql(params: QueryRawEventsParams) -> Result<String, String> {
    let table_name = get_events_table_name();
    let mut conditions = vec![
        format!("tenant_id = '{}'", params.tenant_id),
        format!("timestamp >= '{}'", params.from.format("%Y-%m-%d %H:%M:%S")),
    ];

    // Add time range filter
    if let Some(to) = params.to {
        conditions.push(format!("timestamp < '{}'", to.format("%Y-%m-%d %H:%M:%S")));
    } else {
        conditions.push(format!(
            "timestamp < '{}'",
            Utc::now().format("%Y-%m-%d %H:%M:%S")
        ));
    }

    if !params.customer_ids.is_empty() {
        let customer_ids_str = params
            .customer_ids
            .iter()
            .map(|id| format!("'{}'", id.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        conditions.push(format!("customer_id IN ({customer_ids_str})"));
    }

    if !params.event_codes.is_empty() {
        let event_codes_str = params
            .event_codes
            .iter()
            .map(|code| format!("'{}'", code.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        conditions.push(format!("code IN ({event_codes_str})"));
    }

    if let Some(search) = params.search {
        let escaped_search = search.replace('\'', "''");
        let search_condition = format!(
            "(id ILIKE '%{escaped_search}%' OR code ILIKE '%{escaped_search}%' OR arrayStringConcat(mapValues(properties), ' ') ILIKE '%{escaped_search}%')"
        );
        conditions.push(search_condition);
    }

    let order_by = match params.sort_order {
        EventSortOrder::TimestampDesc => "timestamp DESC",
        EventSortOrder::TimestampAsc => "timestamp ASC",
        EventSortOrder::IngestedDesc => "ingested_at DESC",
        EventSortOrder::IngestedAsc => "ingested_at ASC",
    };

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let query = format!(
        "SELECT
            id,
            code,
            customer_id,
            tenant_id,
            timestamp,
            ingested_at,
            properties
        FROM {}
        {}
        ORDER BY {}
        LIMIT {}
        OFFSET {}",
        table_name, where_clause, order_by, params.limit, params.offset
    );

    Ok(query)
}

pub fn query_meter_view_sql(params: QueryMeterParams) -> Result<String, String> {
    let table_name = get_events_table_name();
    let escaped_namespace = escape_sql_identifier(&params.namespace);
    let escaped_code = escape_sql_identifier(&params.code);

    // Step 1: Build WHERE conditions for subquery
    let mut subquery_conditions = vec![
        format!("tenant_id = '{}'", escaped_namespace),
        format!("code = '{}'", escaped_code),
        format!("timestamp >= toDateTime({})", params.from.timestamp()),
    ];

    if let Some(to) = params.to {
        subquery_conditions.push(format!("timestamp <= toDateTime({})", to.timestamp()));
    }

    if !params.customer_ids.is_empty() {
        let customer_ids_condition = params
            .customer_ids
            .iter()
            .map(|id| format!("customer_id = '{}'", escape_sql_identifier(id)))
            .collect::<Vec<_>>()
            .join(" OR ");
        subquery_conditions.push(format!("({})", customer_ids_condition));
    }

    // Create deduplicated events subquery
    // Deduplicates by (id, customer_id), picking the latest record by timestamp using LIMIT 1 BY
    let dedup_subquery = format!(
        r#"(
        SELECT
            id,
            customer_id,
            timestamp,
            properties
        FROM {}
        WHERE {}
        ORDER BY timestamp DESC
        LIMIT 1 BY id, customer_id
    )"#,
        table_name,
        subquery_conditions.join("\n            AND ")
    );

    // Step 2: Build the main query
    let mut select_columns = Vec::new();
    let mut group_by_columns = Vec::new();
    let mut where_clauses = Vec::new();

    let tz = params
        .window_time_zone
        .as_ref()
        .unwrap_or(&"UTC".to_string())
        .clone();

    // Window columns for SELECT and GROUP BY
    if let Some(window_size) = &params.window_size {
        let (tumble_start, tumble_end) = match window_size {
            WindowSize::Minute => (
                format!("tumbleStart(toDateTime(timestamp), toIntervalMinute(1), '{tz}')"),
                format!("tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), '{tz}')"),
            ),
            WindowSize::Hour => (
                format!("tumbleStart(toDateTime(timestamp), toIntervalHour(1), '{tz}')"),
                format!("tumbleEnd(toDateTime(timestamp), toIntervalHour(1), '{tz}')"),
            ),
            WindowSize::Day => (
                format!("tumbleStart(toDateTime(timestamp), toIntervalDay(1), '{tz}')"),
                format!("tumbleEnd(toDateTime(timestamp), toIntervalDay(1), '{tz}')"),
            ),
        };

        select_columns.push(format!("{} AS window_start", tumble_start));
        select_columns.push(format!("{} AS window_end", tumble_end.clone()));
        group_by_columns.push(tumble_start);
        group_by_columns.push(tumble_end);
    } else {
        select_columns.push("min(toDateTime(timestamp)) AS window_start".to_string());
        select_columns.push("max(toDateTime(timestamp)) AS window_end".to_string());
    }

    // Determine the value expression based on value_property
    let value_expr = if let Some(ref value_prop) = params.value_property {
        let escaped_prop = escape_sql_identifier(value_prop);
        format!("toFloat64OrZero(properties['{}'])", escaped_prop)
    } else if matches!(params.aggregation, MeterAggregation::Count) {
        "1".to_string() // For COUNT aggregation without value property
    } else {
        return Err("value_property is required for non-Count aggregations".to_string());
    };

    // Aggregation function
    let aggregation_column = match &params.aggregation {
        MeterAggregation::Sum => format!("sum({}) AS value", value_expr),
        MeterAggregation::Avg => format!("avg({}) AS value", value_expr),
        MeterAggregation::Min => format!("min({}) AS value", value_expr),
        MeterAggregation::Max => format!("max({}) AS value", value_expr),
        MeterAggregation::Count => "toFloat64(count(*)) AS value".to_string(),
        MeterAggregation::Latest => {
            format!("argMax({}, toDateTime(timestamp)) AS value", value_expr)
        }
        MeterAggregation::CountDistinct => {
            if let Some(ref value_prop) = params.value_property {
                let escaped_prop = escape_sql_identifier(value_prop);
                format!("toFloat64(uniq(properties['{escaped_prop}'])) AS value")
            } else {
                return Err("value_property is required for CountDistinct aggregation".to_string());
            }
        }
    };
    select_columns.push(aggregation_column);

    // Handle customer_ids filtering and grouping
    if !params.customer_ids.is_empty() {
        let subjects_condition = params
            .customer_ids
            .iter()
            .map(|id| format!("customer_id = '{}'", escape_sql_identifier(id)))
            .collect::<Vec<_>>()
            .join(" OR ");
        where_clauses.push(format!("({subjects_condition})"));

        // Add to group by if not already in user's group_by
        if !params.group_by.contains(&"customer_id".to_string()) {
            group_by_columns.push("customer_id".to_string());
        }
        select_columns.push("customer_id".to_string());
    }

    // Handle user-specified group by columns
    for column in &params.group_by {
        let col = PropertyColumn(column);
        group_by_columns.push(col.path());
        select_columns.push(col.as_select());
    }

    // Handle segmentation filters
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
                            let escaped_val = escape_sql_identifier(value);
                            format!("{} = '{escaped_val}'", col.path())
                        })
                        .collect::<Vec<_>>()
                        .join(" OR ");
                    where_clauses.push(format!("({column_condition})"));
                    group_by_columns.push(col.path());
                    select_columns.push(col.as_select());
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
                    let escaped_dim1_val = escape_sql_identifier(dim1_value);
                    if dim2_values.is_empty() {
                        linked_conditions.push(format!("{} = '{escaped_dim1_val}'", col1.path()));
                    } else {
                        let dim2_condition = dim2_values
                            .iter()
                            .map(|v| {
                                let escaped_v = escape_sql_identifier(v);
                                format!("'{escaped_v}'")
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        linked_conditions.push(format!(
                            "({} = '{escaped_dim1_val}' AND {} IN ({dim2_condition}))",
                            col1.path(),
                            col2.path()
                        ));
                    }
                }

                if !linked_conditions.is_empty() {
                    where_clauses.push(format!("({})", linked_conditions.join(" OR ")));
                    group_by_columns.push(col1.path());
                    group_by_columns.push(col2.path());
                    select_columns.push(col1.as_select());
                    select_columns.push(col2.as_select());
                }
            }
        }
    }

    // Add value property filter for non-Count aggregations
    if let Some(ref value_prop) = params.value_property
        && !matches!(params.aggregation, MeterAggregation::Count)
    {
        let col = PropertyColumn(value_prop);
        where_clauses.push(format!(
            "{} != '' AND isNotNull(toFloat64OrNull({}))",
            col.path(),
            col.path()
        ));
    }

    // Construct the final SQL query with subquery
    let mut sql = format!("SELECT {}", select_columns.join(", "));
    sql.push_str(&format!(" FROM {}", dedup_subquery));

    if !where_clauses.is_empty() {
        sql.push_str(&format!(" WHERE {}", where_clauses.join(" AND ")));
    }
    if !group_by_columns.is_empty() {
        sql.push_str(&format!(" GROUP BY {}", group_by_columns.join(", ")));
    }
    if params.window_size.is_some() {
        sql.push_str(" ORDER BY window_start");
    }

    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{MeterAggregation, QueryMeterParams, SegmentationFilter, WindowSize};
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;

    fn normalize_sql(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn test_query_meter_dedup_with_minute_window() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Minute),
            window_time_zone: Some("UTC".to_string()),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1), 'UTC') AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), 'UTC') AS window_end,
                sum(toFloat64OrZero(properties['amount'])) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'test_event'
                    AND timestamp >= toDateTime(1704067200)
                    AND timestamp <= toDateTime(1704153600)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE properties['amount'] != ''
                AND isNotNull(toFloat64OrNull(properties['amount']))
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1), 'UTC'),
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1), 'UTC')
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_count() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Count,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
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
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'api_call'
                    AND timestamp >= toDateTime(1704067200)
                    AND timestamp <= toDateTime(1704153600)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_customer_filter() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "usage".to_string(),
            value_property: Some("bytes".to_string()),
            customer_ids: vec!["cust1".to_string(), "cust2".to_string()],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Hour),
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalHour(1), 'UTC') AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalHour(1), 'UTC') AS window_end,
                sum(toFloat64OrZero(properties['bytes'])) AS value,
                customer_id
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'usage'
                    AND timestamp >= toDateTime(1704067200)
                    AND (customer_id = 'cust1' OR customer_id = 'cust2')
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE (customer_id = 'cust1' OR customer_id = 'cust2')
                AND properties['bytes'] != ''
                AND isNotNull(toFloat64OrNull(properties['bytes']))
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalHour(1), 'UTC'),
                tumbleEnd(toDateTime(timestamp), toIntervalHour(1), 'UTC'),
                customer_id
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_group_by() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Avg,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), 'UTC') AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), 'UTC') AS window_end,
                avg(toFloat64OrZero(properties['duration'])) AS value,
                properties['region'] AS _prop_region,
                properties['endpoint'] AS _prop_endpoint
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'transaction'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE properties['duration'] != ''
                AND isNotNull(toFloat64OrNull(properties['duration']))
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), 'UTC'),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), 'UTC'),
                properties['region'],
                properties['endpoint']
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_independent_segmentation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties['amount'])) AS value,
                properties['region'] AS _prop_region,
                properties['tier'] AS _prop_tier
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'sale'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE (properties['region'] = 'us-east' OR properties['region'] = 'us-west')
                AND (properties['tier'] = 'premium')
                AND properties['amount'] != ''
                AND isNotNull(toFloat64OrNull(properties['amount']))
            GROUP BY properties['region'], properties['tier']
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
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
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();

        // HashMap iteration order is not guaranteed, so check both possible orders
        let expected1 = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties['count'])) AS value,
                properties['product'] AS _prop_product,
                properties['version'] AS _prop_version
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'usage'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE ((properties['product'] = 'prod1' AND properties['version'] IN ('v1', 'v2'))
                   OR (properties['product'] = 'prod2' AND properties['version'] IN ('v3')))
                AND properties['count'] != ''
                AND isNotNull(toFloat64OrNull(properties['count']))
            GROUP BY properties['product'], properties['version']
        "#;

        let expected2 = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties['count'])) AS value,
                properties['product'] AS _prop_product,
                properties['version'] AS _prop_version
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'usage'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE ((properties['product'] = 'prod2' AND properties['version'] IN ('v3'))
                   OR (properties['product'] = 'prod1' AND properties['version'] IN ('v1', 'v2')))
                AND properties['count'] != ''
                AND isNotNull(toFloat64OrNull(properties['count']))
            GROUP BY properties['product'], properties['version']
        "#;

        let result_normalized = normalize_sql(&result);
        assert!(
            result_normalized == normalize_sql(expected1)
                || result_normalized == normalize_sql(expected2),
            "SQL did not match either expected variant.\nGot:\n{}\n\nExpected either:\n{}\n\nor:\n{}",
            result,
            expected1,
            expected2
        );
    }

    #[test]
    fn test_query_meter_dedup_with_count_distinct() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::CountDistinct,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), 'UTC') AS window_start,
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), 'UTC') AS window_end,
                toFloat64(uniq(properties['user_id'])) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'login'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE properties['user_id'] != ''
                AND isNotNull(toFloat64OrNull(properties['user_id']))
            GROUP BY
                tumbleStart(toDateTime(timestamp), toIntervalDay(1), 'UTC'),
                tumbleEnd(toDateTime(timestamp), toIntervalDay(1), 'UTC')
            ORDER BY window_start
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_latest_aggregation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Latest,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                argMax(toFloat64OrZero(properties['value']), toDateTime(timestamp)) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'status'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE properties['value'] != ''
                AND isNotNull(toFloat64OrNull(properties['value']))
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_validates_empty_segmentation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Empty filter for dimension: region");
    }

    #[test]
    fn test_query_meter_dedup_validates_value_property_required() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params);
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
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties['price'])) AS value
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'purchase'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE properties['price'] != ''
                AND isNotNull(toFloat64OrNull(properties['price']))
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_time_filters_in_cte() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Count,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
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
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'event'
                    AND timestamp >= toDateTime(1704067200)
                    AND timestamp <= toDateTime(1704153600)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_reserved_column_in_group_by() {
        // Test that when grouping by a property named "code" (same as reserved column),
        // it uses properties map and aliases as "prop_code" to avoid collision
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "purchase".to_string(),
            value_property: Some("amount".to_string()),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["code".to_string()], // Property named "code" - collides with reserved column
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties['amount'])) AS value,
                properties['code'] AS _prop_code
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'purchase'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE properties['amount'] != ''
                AND isNotNull(toFloat64OrNull(properties['amount']))
            GROUP BY properties['code']
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_with_customer_id_in_segmentation() {
        // Test that when filtering/grouping by a property named "customer_id" (same as reserved column),
        // it uses properties map and aliases as "prop_customer_id" to avoid collision
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
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

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                sum(toFloat64OrZero(properties['bytes'])) AS value,
                properties['customer_id'] AS _prop_customer_id
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'usage'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            WHERE (properties['customer_id'] = 'cust1' OR properties['customer_id'] = 'cust2')
                AND properties['bytes'] != ''
                AND isNotNull(toFloat64OrNull(properties['bytes']))
            GROUP BY properties['customer_id']
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }

    #[test]
    fn test_query_meter_dedup_mixed_reserved_and_custom_columns() {
        // Test mix of properties with reserved names and custom properties in group_by
        // Reserved names get prop_ prefix, custom properties use their normal name
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Count,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "event".to_string(),
            value_property: None,
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![
                "customer_id".to_string(), // property with reserved name - gets prop_ prefix
                "region".to_string(),      // custom property - normal alias
                "code".to_string(),        // property with reserved name - gets prop_ prefix
            ],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT
                min(toDateTime(timestamp)) AS window_start,
                max(toDateTime(timestamp)) AS window_end,
                toFloat64(count(*)) AS value,
                properties['customer_id'] AS _prop_customer_id,
                properties['region'] AS _prop_region,
                properties['code'] AS _prop_code
            FROM (
                SELECT
                    id,
                    customer_id,
                    timestamp,
                    properties
                FROM meteroid.raw_events
                WHERE tenant_id = 'test_ns'
                    AND code = 'event'
                    AND timestamp >= toDateTime(1704067200)
                ORDER BY timestamp DESC
                LIMIT 1 BY id, customer_id
            )
            GROUP BY properties['customer_id'], properties['region'], properties['code']
        "#;

        assert_eq!(normalize_sql(&result), normalize_sql(expected));
    }
}
