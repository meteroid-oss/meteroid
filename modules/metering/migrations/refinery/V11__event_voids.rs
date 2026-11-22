use crate::migrations::get_clickhouse_config;

pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;

    format!(
        r#"
        ALTER TABLE meteroid.raw_events_local_v2 ON CLUSTER '{cluster_name}'
            ADD COLUMN IF NOT EXISTS is_tombstone UInt8 DEFAULT 0;

        ALTER TABLE meteroid.raw_events_v2 ON CLUSTER '{cluster_name}'
            ADD COLUMN IF NOT EXISTS is_tombstone UInt8 DEFAULT 0;

        -- A tombstone copies the row it cancels, so readers narrow this set with their own predicates.
        CREATE TABLE IF NOT EXISTS meteroid.event_voids_local ON CLUSTER '{cluster_name}' (
            tenant_id   UUID,
            code        String,
            customer_id UUID,
            id          String,
            timestamp   DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC')
        ) ENGINE = ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/event_voids_local', '{{replica}}')
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, code, customer_id, id);

        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.event_voids_mv ON CLUSTER '{cluster_name}'
        TO meteroid.event_voids_local AS
            SELECT tenant_id, code, customer_id, id, timestamp, ingested_at
            FROM meteroid.raw_events_local_v2
            WHERE is_tombstone = 1
        "#,
        cluster_name = cluster_name,
    )
}
