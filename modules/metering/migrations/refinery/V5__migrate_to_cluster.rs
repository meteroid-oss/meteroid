use crate::migrations::{build_kafka_settings, get_clickhouse_config, get_kafka_config};

pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;
    let database = &clickhouse_cfg.database;

    let kafka_cfg = get_kafka_config();

    let raw_kafka_settings = build_kafka_settings(
        &kafka_cfg.kafka_internal_addr,
        &kafka_cfg.kafka_raw_topic,
        "clickhouse-events-raw",
        &kafka_cfg.kafka_connection,
    );

    format!(
        r#"
        -- Step 1: Create local storage table with ReplicatedMergeTree
        CREATE TABLE IF NOT EXISTS meteroid.raw_events_local ON CLUSTER '{cluster_name}' (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/raw_events_local', '{{replica}}')
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, timestamp, code, customer_id);

        -- Step 2: Copy existing data (safe: INSERT before any DROP)
        INSERT INTO meteroid.raw_events_local SELECT * FROM meteroid.raw_events;

        -- Step 3: Tear down the Kafka pipeline (MV must go first)
        DROP TABLE IF EXISTS meteroid.raw_kafka_events_mv ON CLUSTER '{cluster_name}';
        DROP TABLE IF EXISTS meteroid.raw_kafka_events ON CLUSTER '{cluster_name}';

        -- Step 4: Drop old non-distributed raw_events
        DROP TABLE IF EXISTS meteroid.raw_events ON CLUSTER '{cluster_name}';

        -- Step 5: Create Distributed routing table (sharded by tenant for query locality)
        CREATE TABLE IF NOT EXISTS meteroid.raw_events ON CLUSTER '{cluster_name}' (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = Distributed('{cluster_name}', 'meteroid', 'raw_events_local', cityHash64(tenant_id));

        -- Step 6: Recreate Kafka consumer table
        CREATE TABLE IF NOT EXISTS meteroid.raw_kafka_events ON CLUSTER '{cluster_name}' (
            id String,
            code String,
            customer_id String,
            tenant_id String,
            timestamp DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties Map(String, String)
        ) ENGINE = Kafka()
          SETTINGS
              {raw_kafka_settings};

        -- Step 7: Recreate MV writing to local table (never write through Distributed)
        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.raw_kafka_events_mv ON CLUSTER '{cluster_name}'
        TO meteroid.raw_events_local AS
        SELECT * FROM meteroid.raw_kafka_events;

        -- Step 8: Migrate refinery tracking table to Distributed+local pattern

        CREATE TABLE IF NOT EXISTS refinery_schema_history_local ON CLUSTER '{cluster_name}'
        (version INT, name String, applied_on String, checksum String)
        ENGINE = ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/refinery_schema_history_local', '{{replica}}')
        ORDER BY version;

        INSERT INTO refinery_schema_history_local
        SELECT * FROM refinery_schema_history
        WHERE version NOT IN (SELECT version FROM refinery_schema_history_local);

        CREATE TABLE IF NOT EXISTS refinery_schema_history_new ON CLUSTER '{cluster_name}'
        (version INT, name String, applied_on String, checksum String)
        ENGINE = Distributed('{cluster_name}', '{database}', 'refinery_schema_history_local', rand());

        EXCHANGE TABLES refinery_schema_history AND refinery_schema_history_new ON CLUSTER '{cluster_name}';

        DROP TABLE IF EXISTS refinery_schema_history_new ON CLUSTER '{cluster_name}';
    "#,
        cluster_name = cluster_name,
        database = database,
        raw_kafka_settings = raw_kafka_settings,
    )
}
