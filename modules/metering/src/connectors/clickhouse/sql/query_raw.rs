use crate::connectors::clickhouse::sql::init::get_events_table_name;
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
        "SELECT event_id, event_name, customer_id, event_timestamp, properties FROM {}",
        table_name
    );

    where_clauses.push(format!("tenant_id = '{}'", tenant_id));

    if let Some(from_time) = from {
        where_clauses.push(format!("event_timestamp >= {}", from_time.timestamp()));
    }

    if let Some(to_time) = to {
        where_clauses.push(format!("event_timestamp <= {}", to_time.timestamp()));
    }

    if !where_clauses.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&where_clauses.join(" AND "));
    }

    query.push_str(&format!(" ORDER BY time DESC LIMIT {}", limit));

    query
}
