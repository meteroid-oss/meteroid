use crate::migrations::get_clickhouse_config;
pub fn migration() -> String {
    let clickhouse_cfg = get_clickhouse_config();
    let cluster_name = &clickhouse_cfg.cluster_name;
    let cluster_clause = format!(" ON CLUSTER '{cluster_name}'");
    format!(
        r#"
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events_mv{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_kafka_events{cluster_clause};
        DROP TABLE IF EXISTS meteroid.preprocessed_events{cluster_clause};
    "#
    )
}
