use crate::migrations::get_clickhouse_config;
pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    // Conditionally use ON CLUSTER based on configuration
    let cluster_clause = if let Some(ref cluster_name) = clickhouse_cfg.cluster_name {
        format!(" ON CLUSTER '{}'", cluster_name)
    } else {
        String::new()
    };
    format!(
        r#"
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events_mv{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_events{cluster_clause};
    "#
    )
}
