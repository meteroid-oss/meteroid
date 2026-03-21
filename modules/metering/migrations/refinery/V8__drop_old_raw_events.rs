use crate::migrations::get_clickhouse_config;

pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;

    format!(
        r#"
        DROP TABLE IF EXISTS meteroid.raw_events ON CLUSTER '{cluster_name}';
        DROP TABLE IF EXISTS meteroid.raw_events_local ON CLUSTER '{cluster_name}';
        "#,
        cluster_name = cluster_name,
    )
}
