use crate::connectors::clickhouse::sql::DATABASE;

pub fn get_events_table_name() -> String {
    format!("{}.{}", DATABASE, "raw_events")
}
