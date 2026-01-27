use crate::connectors::clickhouse::sql::{
    PropertyColumn, escape_sql_identifier, get_meter_view_name,
};
use crate::domain::{MeterAggregation, QueryMeterParams, SegmentationFilter, WindowSize};

pub fn query_meter_view_sql(params: QueryMeterParams) -> Result<String, String> {
    let view_name = get_meter_view_name(&params.namespace, &params.meter_slug);

    let mut select_columns = Vec::new();
    let mut group_by_columns = Vec::new();
    let mut where_clauses = Vec::new();

    // TODO
    let tz = params
        .window_time_zone
        .as_ref()
        .unwrap_or(&"UTC".to_string())
        .clone();

    if let Some(window_size) = &params.window_size {
        match window_size {
            WindowSize::Minute => {
                select_columns.push(format!(
                    "tumbleStart(windowstart, toIntervalMinute(1), '{tz}') AS window_start"
                ));
                select_columns.push(format!(
                    "tumbleEnd(windowstart, toIntervalMinute(1), '{tz}') AS window_end"
                ));
            }
            WindowSize::Hour => {
                select_columns.push(format!(
                    "tumbleStart(windowstart, toIntervalHour(1), '{tz}') AS window_start"
                ));
                select_columns.push(format!(
                    "tumbleEnd(windowstart, toIntervalHour(1), '{tz}') AS window_end"
                ));
            }
            WindowSize::Day => {
                select_columns.push(format!(
                    "tumbleStart(windowstart, toIntervalDay(1), '{tz}') AS window_start"
                ));
                select_columns.push(format!(
                    "tumbleEnd(windowstart, toIntervalDay(1), '{tz}') AS window_end"
                ));
            }
        }
        group_by_columns.push("windowstart".to_string());
        group_by_columns.push("windowend".to_string());
    } else {
        select_columns.push("min(windowstart) AS window_start".to_string());
        select_columns.push("max(windowend) AS window_end".to_string());
    }

    let aggregation_column = match &params.aggregation {
        MeterAggregation::Sum => "sumMerge(value) AS value",
        MeterAggregation::Avg => "avgMerge(value) AS value",
        MeterAggregation::Min => "minMerge(value) AS value",
        MeterAggregation::Max => "maxMerge(value) AS value",
        MeterAggregation::Count => "toFloat64(countMerge(value)) AS value",
        // TODO
        MeterAggregation::Latest => "argMaxMerge(value, windowstart) AS value",
        MeterAggregation::CountDistinct => {
            // let mut columns = Vec::new();
            // for (column, values) in &params.filter_group_by {
            //     if values.is_empty() {
            //         return Err(format!("Empty filter for group by: {}", column));
            //     }
            //     let column_condition = values.iter()
            //         .map(|value| format!("{} = '{}'", column, value))
            //         .collect::<Vec<_>>().join(" OR ");
            //     columns.push(format!("uniqMergeIf({}, ({})) AS {}", column, column_condition, column));
            // }
            // columns.push("uniqMerge(customer_id) AS customer_id".to_string());
            // columns.join(", ")
            return Err("CountDistinct not implemented".to_string());
        }
    };
    select_columns.push(aggregation_column.to_string());

    // Add customer_id if we have customer filtering and it's not already in group_by
    if !params.customer_ids.is_empty() && !params.group_by.contains(&"customer_id".to_string()) {
        group_by_columns.push("customer_id".to_string());
    }

    // Add user-specified group by columns
    for column in &params.group_by {
        let col = PropertyColumn(column);
        group_by_columns.push(col.path());
        select_columns.push(col.as_select());
    }

    if !params.customer_ids.is_empty() {
        let subjects_condition = params
            .customer_ids
            .iter()
            .map(|id| format!("customer_id = '{}'", escape_sql_identifier(id)))
            .collect::<Vec<_>>()
            .join(" OR ");
        where_clauses.push(format!("({subjects_condition})"));
        select_columns.push("customer_id".to_string());
    }

    if let Some(ref segmentation) = params.segmentation_filter {
        match segmentation {
            SegmentationFilter::Independent(filters) => {
                // Independent filters are ANDed together
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
                // Linked filters create an OR condition for each linked pair
                let mut linked_conditions = Vec::new();
                let col1 = PropertyColumn(dimension1_key);
                let col2 = PropertyColumn(dimension2_key);

                for (dim1_value, dim2_values) in values {
                    let escaped_dim1_val = escape_sql_identifier(dim1_value);
                    if dim2_values.is_empty() {
                        // If no dim2 values, just filter on dim1
                        linked_conditions.push(format!("{} = '{escaped_dim1_val}'", col1.path()));
                    } else {
                        // Create condition: (dim1 = 'val1' AND dim2 IN ('val2a', 'val2b'))
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
                    // Combine all linked conditions with OR
                    where_clauses.push(format!("({})", linked_conditions.join(" OR ")));
                    group_by_columns.push(col1.path());
                    group_by_columns.push(col2.path());
                    select_columns.push(col1.as_select());
                    select_columns.push(col2.as_select());
                }
            }
        }
    }

    // Time filter clauses
    // TODO limit & probably make from required
    where_clauses.push(format!("windowstart >= {}", params.from.timestamp()));
    if let Some(to) = params.to {
        where_clauses.push(format!("windowend <= {}", to.timestamp()));
    }

    // Constructing the final SQL query
    let mut sql = format!("SELECT {} FROM {}", select_columns.join(", "), view_name);
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
    use chrono::{TimeZone, Utc};
    use std::collections::HashMap;

    fn clean_sql(sql: &str) -> String {
        sql.replace(['\n', ' '], "")
    }

    #[test]
    fn test_query_meter_view_with_minute_window() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Minute),
            window_time_zone: Some("UTC".to_string()),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT tumbleStart(windowstart, toIntervalMinute(1), 'UTC') AS window_start,
                   tumbleEnd(windowstart, toIntervalMinute(1), 'UTC') AS window_end,
                   sumMerge(value) AS value
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
              AND windowend <= 1704153600
            GROUP BY windowstart, windowend
            ORDER BY window_start
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_hour_window() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Avg,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Hour),
            window_time_zone: Some("America/New_York".to_string()),
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT tumbleStart(windowstart, toIntervalHour(1), 'America/New_York') AS window_start,
                   tumbleEnd(windowstart, toIntervalHour(1), 'America/New_York') AS window_end,
                   avgMerge(value) AS value
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
            GROUP BY windowstart, windowend
            ORDER BY window_start
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_day_window() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Max,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 31, 0, 0, 0).unwrap()),
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT tumbleStart(windowstart, toIntervalDay(1), 'UTC') AS window_start,
                   tumbleEnd(windowstart, toIntervalDay(1), 'UTC') AS window_end,
                   maxMerge(value) AS value
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
              AND windowend <= 1706659200
            GROUP BY windowstart, windowend
            ORDER BY window_start
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_without_window() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Count,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()),
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   toFloat64(countMerge(value)) AS value
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
              AND windowend <= 1704153600
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_customer_ids() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec!["cust1".to_string(), "cust2".to_string()],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   sumMerge(value) AS value,
                   customer_id
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE (customer_id = 'cust1' OR customer_id = 'cust2')
              AND windowstart >= 1704067200
            GROUP BY customer_id
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_group_by() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec!["region".to_string(), "product".to_string()],
            window_size: Some(WindowSize::Day),
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT tumbleStart(windowstart, toIntervalDay(1), 'UTC') AS window_start,
                   tumbleEnd(windowstart, toIntervalDay(1), 'UTC') AS window_end,
                   sumMerge(value) AS value,
                   properties['region'] AS _prop_region,
                   properties['product'] AS _prop_product
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
            GROUP BY windowstart, windowend, properties['region'], properties['product']
            ORDER BY window_start
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_independent_segmentation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
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
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   sumMerge(value) AS value,
                   properties['region'] AS _prop_region,
                   properties['tier'] AS _prop_tier
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE (properties['region'] = 'us-east' OR properties['region'] = 'us-west')
              AND (properties['tier'] = 'premium')
              AND windowstart >= 1704067200
            GROUP BY properties['region'], properties['tier']
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_linked_segmentation() {
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
            code: "test_event".to_string(),
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
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        // Note: HashMap iteration order is not guaranteed, so we need to check both possible orders
        let result_clean = clean_sql(&result);

        let expected1 = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   sumMerge(value) AS value,
                   properties['product'] AS _prop_product,
                   properties['version'] AS _prop_version
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE ((properties['product'] = 'prod1' AND properties['version'] IN ('v1', 'v2'))
                   OR (properties['product'] = 'prod2' AND properties['version'] IN ('v3')))
              AND windowstart >= 1704067200
            GROUP BY properties['product'], properties['version']
        "#;

        let expected2 = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   sumMerge(value) AS value,
                   properties['product'] AS _prop_product,
                   properties['version'] AS _prop_version
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE ((properties['product'] = 'prod2' AND properties['version'] IN ('v3'))
                   OR (properties['product'] = 'prod1' AND properties['version'] IN ('v1', 'v2')))
              AND windowstart >= 1704067200
            GROUP BY properties['product'], properties['version']
        "#;

        assert!(
            result_clean == clean_sql(expected1) || result_clean == clean_sql(expected2),
            "Result did not match either expected pattern.\nGot: {}\nExpected either:\n{}\nor:\n{}",
            result_clean,
            clean_sql(expected1),
            clean_sql(expected2)
        );
    }

    #[test]
    fn test_query_meter_view_with_min_aggregation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Min,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   minMerge(value) AS value
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_latest_aggregation() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Latest,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   argMaxMerge(value, windowstart) AS value
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE windowstart >= 1704067200
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_customer_and_group_by() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec!["cust1".to_string()],
            segmentation_filter: None,
            group_by: vec!["region".to_string()],
            window_size: Some(WindowSize::Hour),
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: Some(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap()),
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT tumbleStart(windowstart, toIntervalHour(1), 'UTC') AS window_start,
                   tumbleEnd(windowstart, toIntervalHour(1), 'UTC') AS window_end,
                   sumMerge(value) AS value,
                   properties['region'] AS _prop_region,
                   customer_id
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE (customer_id = 'cust1')
              AND windowstart >= 1704067200
              AND windowend <= 1704110400
            GROUP BY windowstart, windowend, customer_id, properties['region']
            ORDER BY window_start
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_with_customer_in_group_by() {
        // When customer_id is already in group_by, it shouldn't be added again
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec!["cust1".to_string()],
            segmentation_filter: None,
            group_by: vec!["customer_id".to_string()],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params).unwrap();
        let expected = r#"
            SELECT min(windowstart) AS window_start,
                   max(windowend) AS window_end,
                   sumMerge(value) AS value,
                   properties['customer_id'] AS _prop_customer_id,
                   customer_id
            FROM meteroid.METER_NStestns_Mtestmeter
            WHERE (customer_id = 'cust1')
              AND windowstart >= 1704067200
            GROUP BY properties['customer_id']
        "#;

        assert_eq!(clean_sql(&result), clean_sql(expected));
    }

    #[test]
    fn test_query_meter_view_empty_segmentation_filter_error() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::Sum,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: Some(SegmentationFilter::Independent(vec![
                ("region".to_string(), vec![]), // Empty filter values
            ])),
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        let result = query_meter_view_sql(params);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Empty filter for dimension: region");
    }

    #[test]
    fn test_query_meter_view_count_distinct_not_implemented() {
        let params = QueryMeterParams {
            aggregation: MeterAggregation::CountDistinct,
            namespace: "test_ns".to_string(),
            meter_slug: "test_meter".to_string(),
            code: "test_event".to_string(),
            customer_ids: vec![],
            segmentation_filter: None,
            group_by: vec![],
            window_size: None,
            window_time_zone: None,
            from: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            to: None,
            value_property: None,
        };

        assert_eq!(
            query_meter_view_sql(params),
            Err("CountDistinct not implemented".to_string())
        );
    }
}
