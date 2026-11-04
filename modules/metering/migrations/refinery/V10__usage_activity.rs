use crate::migrations::get_clickhouse_config;

pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;

    format!(
        r#"
        CREATE TABLE IF NOT EXISTS meteroid.usage_activity_local ON CLUSTER '{cluster_name}' (
            tenant_id   UUID,
            customer_id UUID,
            seen_at     DateTime64(3, 'UTC'),
            min_event_at DateTime64(3, 'UTC')
        ) ENGINE = ReplicatedMergeTree('/clickhouse/tables/{{cluster}}/{{database}}/usage_activity_local', '{{replica}}')
          PARTITION BY toDate(seen_at)
          ORDER BY (seen_at, tenant_id, customer_id)
          TTL toDateTime(seen_at) + INTERVAL 1 DAY;

        CREATE TABLE IF NOT EXISTS meteroid.usage_activity ON CLUSTER '{cluster_name}' (
            tenant_id   UUID,
            customer_id UUID,
            seen_at     DateTime64(3, 'UTC'),
            min_event_at DateTime64(3, 'UTC')
        ) ENGINE = Distributed('{cluster_name}', 'meteroid', 'usage_activity_local', cityHash64(tenant_id));

        CREATE MATERIALIZED VIEW IF NOT EXISTS meteroid.usage_activity_mv ON CLUSTER '{cluster_name}'
        TO meteroid.usage_activity_local
        AS SELECT tenant_id, customer_id, now64(3) AS seen_at,
                  toDateTime64(min(timestamp), 3, 'UTC') AS min_event_at
           FROM meteroid.raw_events_local_v2
           GROUP BY tenant_id, customer_id
        "#,
        cluster_name = cluster_name,
    )
}
