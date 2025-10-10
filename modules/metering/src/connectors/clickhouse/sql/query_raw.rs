use crate::connectors::clickhouse::sql::init::get_events_table_name;
use crate::domain::{EventSortOrder, QueryRawEventsParams};
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
