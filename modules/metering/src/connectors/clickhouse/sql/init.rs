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
const COMMON_COLUMNS: &str = "id String,
    code String,
    customer_id String,
    tenant_id String,
    timestamp DateTime64(9, 'UTC'),
    ingested_at DateTime64(9, 'UTC'),
    properties Map(String, String)";

pub(crate) fn create_events_table_sql() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (
            {}
        ) ENGINE = MergeTree
        PARTITION BY toYYYYMM(timestamp)
        ORDER BY (tenant_id, timestamp, code, customer_id)",
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
    kafka_broker_list: &str,
    kafka_topic_list: &str,
    kafka_group_name: &str,
    kafka_format: &str,
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
        kafka_broker_list,
        kafka_topic_list,
        kafka_group_name,
        kafka_format,
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
