use crate::migrations::get_clickhouse_config;

pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;

    format!(
        r#"
        CREATE TABLE IF NOT EXISTS meteroid.raw_events_local_v2 ON CLUSTER '{cluster_name}' (
            id          String,
            code        String,
            customer_id UUID,
            tenant_id   UUID,
            timestamp   DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties  Map(String, String),
            INDEX idx_timestamp timestamp TYPE minmax GRANULARITY 1
        ) ENGINE = ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/raw_events_local_v2', '{{replica}}')
          PARTITION BY toYYYYMM(timestamp)
          ORDER BY (tenant_id, code, timestamp, customer_id);

        CREATE TABLE IF NOT EXISTS meteroid.raw_events_v2 ON CLUSTER '{cluster_name}' (
            id          String,
            code        String,
            customer_id UUID,
            tenant_id   UUID,
            timestamp   DateTime64(9, 'UTC'),
            ingested_at DateTime64(9, 'UTC'),
            properties  Map(String, String)
        ) ENGINE = Distributed('{cluster_name}', 'meteroid', 'raw_events_local_v2', cityHash64(tenant_id))
        "#,
        cluster_name = cluster_name,
    )
}
