use crate::connectors::clickhouse::sql::get_meter_view_name;
use crate::domain::{MeterAggregation, QueryMeterParams, WindowSize};

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
                    "tumbleStart(windowstart, toIntervalMinute(1), '{}') AS window_start",
                    tz
                ));
                select_columns.push(format!(
                    "tumbleEnd(windowstart, toIntervalMinute(1), '{}') AS window_end",
                    tz
                ));
            }
            WindowSize::Hour => {
                select_columns.push(format!(
                    "tumbleStart(windowstart, toIntervalHour(1), '{}') AS window_start",
                    tz
                ));
                select_columns.push(format!(
                    "tumbleEnd(windowstart, toIntervalHour(1), '{}') AS window_end",
                    tz
                ));
            }
            WindowSize::Day => {
                select_columns.push(format!(
                    "tumbleStart(windowstart, toIntervalDay(1), '{}') AS window_start",
                    tz
                ));
                select_columns.push(format!(
                    "tumbleEnd(windowstart, toIntervalDay(1), '{}') AS window_end",
                    tz
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
            unimplemented!("CountDistinct not implemented")
        }
    };
    select_columns.push(aggregation_column.to_string());

    let mut group_by_columns = params.group_by.clone();
    // Add group by columns
    if !params.customer_ids.is_empty() && !params.group_by.contains(&"customer_id".to_string()) {
        group_by_columns.insert(0, "customer_id".to_string());
    }

    for column in &params.group_by {
        group_by_columns.push(column.clone());
        select_columns.push(column.clone());
    }

    if !params.customer_ids.is_empty() {
        let subjects_condition = params
            .customer_ids
            .iter()
            .map(|id| format!("customer_id = '{}'", id)) // TODO config for id/ext/custom field
            .collect::<Vec<_>>()
            .join(" OR ");
        where_clauses.push(format!("({})", subjects_condition));
        select_columns.push("customer_id".to_string());
    }

    // FilterGroupBy clauses
    for (column, values) in &params.filter_group_by {
        if values.is_empty() {
            return Err(format!("Empty filter for group by: {}", column));
        }
        let column_condition = values
            .iter()
            .map(|value| format!("{} = '{}'", column, value))
            .collect::<Vec<_>>()
            .join(" OR ");
        where_clauses.push(format!("({})", column_condition));
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
