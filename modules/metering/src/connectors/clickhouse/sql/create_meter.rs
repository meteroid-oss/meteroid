use crate::connectors::clickhouse::sql::init::get_events_table_name;
use crate::connectors::clickhouse::sql::{Column, escape_sql_identifier, get_meter_view_name};
use crate::domain::{Meter, MeterAggregation};

use std::fmt;

impl fmt::Display for MeterAggregation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeterAggregation::Sum => write!(f, "sum"),
            MeterAggregation::Avg => write!(f, "avg"),
            MeterAggregation::Min => write!(f, "min"),
            MeterAggregation::Max => write!(f, "max"),
            MeterAggregation::Count => write!(f, "count"),
            MeterAggregation::CountDistinct => write!(f, "uniq"),
            MeterAggregation::Latest => write!(f, "argMax"), // TODO unimplemented
        }
    }
}

fn create_meter_view_to_select_sql(meter: Meter) -> String {
    let agg_state_fn = format!("{}State", meter.aggregation);

    // TODO we moved from day to minute aggregation. Not sure if we should keep it like this (or we can make it configurable)
    // Also we want to make sure that we can group by full day, as it's the main view. Maybe an extra MV ?
    let mut selects = vec![
        "customer_id".to_string(),
        "tumbleStart(toDateTime(timestamp), toIntervalMinute(1)) AS windowstart".to_string(),
        "tumbleEnd(toDateTime(timestamp), toIntervalMinute(1)) AS windowend".to_string(),
    ];

    // we rasterize the value property to be an option of non empty string
    let value_property_nes = meter
        .value_property
        .and_then(|v| if v.is_empty() { None } else { Some(v) });

    let where_clause_for_value_property = match value_property_nes {
        Some(ref value_property) => {
            let escaped_prop = escape_sql_identifier(&value_property);
            selects.push(format!(
                "{}(toFloat64OrZero(properties['{}'])) AS value",
                agg_state_fn, escaped_prop
            ));
            format!(
                " AND properties['{}'] != '' AND isNotNull(toFloat64OrNull(properties['{}']))",
                escaped_prop, escaped_prop
            )
        }
        None => {
            if matches!(meter.aggregation, MeterAggregation::Count) {
                selects.push(format!("{agg_state_fn}(*) AS value"));
            } else {
                // TODO should only allow for Count
                unimplemented!("Only Count aggregation is supported without value property")
            }
            String::new()
        }
    };

    let mut order_by = vec![
        "windowstart".to_string(),
        "windowend".to_string(),
        "customer_id".to_string(),
    ];
    let mut sorted_group_by = meter.group_by;
    sorted_group_by.sort();

    for k in &sorted_group_by {
        let column_name = escape_sql_identifier(k);
        order_by.push(escape_sql_identifier(&column_name));
        // selects.push(format!("JSON_VALUE(data, '{}') as {}", escape_sql_identifier(v), escape_sql_identifier(k)));
        selects.push(format!("properties['{column_name}'] as {column_name}"));
    }

    let events_table_name = get_events_table_name();

    let query = format!(
        "SELECT {} FROM {} WHERE {}.tenant_id = '{}' AND {}.code = '{}'{} GROUP BY {}",
        selects.join(", "),
        events_table_name,
        events_table_name,
        escape_sql_identifier(&meter.namespace),
        events_table_name,
        escape_sql_identifier(&meter.code),
        where_clause_for_value_property,
        order_by.join(", "), // TODO check
    );

    query
}

pub fn create_meter_view(meter: Meter, populate: bool) -> String {
    let view_name = get_meter_view_name(&meter.namespace, &meter.id);
    let mut columns = vec![
        Column {
            name: "customer_id".to_string(),
            col_type: "String".to_string(),
        },
        Column {
            name: "windowstart".to_string(),
            col_type: "DateTime".to_string(),
        },
        Column {
            name: "windowend".to_string(),
            col_type: "DateTime".to_string(),
        },
    ];

    // Add value column based on aggregation type
    let agg_column_type = format!("AggregateFunction({}, Float64)", meter.aggregation);
    columns.push(Column {
        name: "value".to_string(),
        col_type: agg_column_type,
    });

    // Add group by columns
    let mut order_by = vec![
        "windowstart".to_string(),
        "windowend".to_string(),
        "customer_id".to_string(),
    ];
    for key in &meter.group_by {
        order_by.push(key.clone());
        columns.push(Column {
            name: key.clone(),
            col_type: "String".to_string(),
        });
    }

    // Construct SQL
    let mut sql = format!("CREATE MATERIALIZED VIEW IF NOT EXISTS {view_name} (\n");
    for col in &columns {
        sql.push_str(&format!("    {} {},\n", col.name, col.col_type));
    }
    sql.pop();
    sql.pop(); // Remove the last comma
    sql.push_str(&format!(
        ") ENGINE = AggregatingMergeTree() ORDER BY ({})\n",
        order_by.join(", ")
    ));

    if populate {
        sql.push_str("POPULATE\n");
    }

    let select_query = create_meter_view_to_select_sql(meter);

    // Add SELECT statement
    sql.push_str(&format!("AS {select_query}\n")); // Add your select statement here

    sql
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Meter, MeterAggregation};

    fn clean_sql(sql: &str) -> String {
        sql.replace("\n", "").replace(" ", "")
    }

    #[test]
    fn test_create_meter_view_count() {
        let meter = Meter {
            namespace: "test_namespace".to_string(),
            id: "test_slug".to_string(),
            code: "test_event".to_string(),
            aggregation: MeterAggregation::Count,
            group_by: vec!["test_group1".to_string(), "test_group2".to_string()],
            value_property: Some("test_value".to_string()),
        };

        let expected = r#"
            CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.METER_NStestnamespace_Mtestslug (
                customer_id String,
                windowstart DateTime,
                windowend DateTime,
                value AggregateFunction(count, Float64),
                test_group1 String,
                test_group2 String)
            ENGINE = AggregatingMergeTree()
            ORDER BY (windowstart, windowend, customer_id, test_group1, test_group2)
            POPULATE
            AS SELECT
                customer_id,
                tumbleStart(toDateTime(timestamp), toIntervalMinute(1)) AS windowstart,
                tumbleEnd(toDateTime(timestamp), toIntervalMinute(1)) AS windowend,
                countState(toFloat64OrZero(properties['test_value'])) AS value,
                properties['test_group1'] as test_group1,
                properties['test_group2'] as test_group2
            FROM meteroid.raw_events
            WHERE meteroid.raw_events.tenant_id = 'test_namespace'
                AND meteroid.raw_events.code = 'test_event'
                AND properties['test_value'] != ''
                AND isNotNull(toFloat64OrNull(properties['test_value']))
                GROUP BY windowstart, windowend, customer_id, test_group1, test_group2
        "#;

        let result = create_meter_view(meter, true);
        // assert equal ignoring whitespace
        assert_eq!(clean_sql(&result), clean_sql(expected));
    }
}
