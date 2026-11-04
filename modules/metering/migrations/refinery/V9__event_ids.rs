use crate::migrations::get_clickhouse_config;

pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;

    format!(
        r#"
        CREATE TABLE IF NOT EXISTS meteroid.event_ids_local ON CLUSTER '{cluster_name}' (
            tenant_id   UUID,
            customer_id UUID,
            id          String,
            timestamp   DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC')
        ) ENGINE = ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/event_ids_local', '{{replica}}')
          PARTITION BY toYYYYMM(ingested_at)
          ORDER BY (tenant_id, customer_id, id)
          TTL toDateTime(ingested_at) + INTERVAL 35 DAY
          SETTINGS index_granularity = 256;

        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.event_ids_mv ON CLUSTER '{cluster_name}'
        TO meteroid.event_ids_local AS
            SELECT tenant_id, customer_id, id, timestamp, ingested_at FROM meteroid.raw_events_local_v2;

        -- Ids of the dedup window ingested before the view existed; replicates to every replica.
        INSERT INTO meteroid.event_ids_local
            SELECT tenant_id, customer_id, id, timestamp, ingested_at FROM meteroid.raw_events_local_v2
            WHERE ingested_at >= now() - INTERVAL 35 DAY
        "#,
        cluster_name = cluster_name,
    )
}
