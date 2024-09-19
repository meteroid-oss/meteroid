use crate::connectors::clickhouse::sql::DATABASE;

const TABLE_PREFIX: &str = "raw"; // TODO ?

fn get_table_name(table_suffix: &str) -> String {
    format!("{}.{}_{}", DATABASE, TABLE_PREFIX, table_suffix)
}

// the stored mergetree events table
pub fn get_events_table_name() -> String {
    get_table_name("events")
}

// the streaming ingestion table, temporary storage
fn get_kafka_events_table_name() -> String {
    get_table_name("kafka_events")
}

// the materialized view writing the ingestion table to the events table
fn get_kafka_mv_table_name() -> String {
    get_table_name("kafka_events_mv")
}

// data String if we want JSON with path, but to simplify for end user let's use Map<String,String> for now
// TODO LowCardinality(String) for tenant, event name and for property key as well when available, https://github.com/suharev7/clickhouse-rs/issues/199#issuecomment-1837427136
const COMMON_COLUMNS: &str = "tenant_id String,
    event_id String,
    event_name String,
    customer_id String,
    event_timestamp DateTime64(9, 'UTC'),
    properties Map(String, String)";

pub(crate) fn create_events_table_sql() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (
            {}
        ) ENGINE = MergeTree
        PARTITION BY toYYYYMM(event_timestamp)
        ORDER BY (tenant_id, event_timestamp, event_name, customer_id)",
        get_events_table_name(),
        COMMON_COLUMNS
    )
}
/*
    TODO
    TTL
       timestamp TO VOLUME 'hot',
       timestamp + INTERVAL 60 DAY TO VOLUME 'cold'
     SETTINGS
       storage_policy = 'hot_cold';
*/

pub(crate) fn create_kafka_event_table_sql(
    kafka_broker_list: String,
    kafka_topic_list: String,
    kafka_group_name: String,
    kafka_format: String,
) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (
                {}
            )ENGINE = Kafka()
            SETTINGS
                kafka_broker_list = '{}',
                kafka_topic_list = '{}',
                kafka_group_name = '{}',
                kafka_format = '{}'",
        get_kafka_events_table_name(),
        COMMON_COLUMNS,
        &kafka_broker_list,
        &kafka_topic_list,
        &kafka_group_name,
        &kafka_format,
    )
}

pub(crate) fn create_kafka_mv_sql() -> String {
    format!(
        "CREATE MATERIALIZED VIEW IF NOT EXISTS {} TO {} AS
            SELECT * FROM {}",
        get_kafka_mv_table_name(),
        get_events_table_name(),
        get_kafka_events_table_name(),
    )
}
