use envconfig::Envconfig;

/// Connection settings for the shared ClickHouse cluster.
#[derive(Envconfig, Clone)]
pub struct ClickhouseConfig {
    #[envconfig(from = "CLICKHOUSE_DATABASE", default = "meteroid")]
    pub database: String,

    #[envconfig(from = "CLICKHOUSE_HTTP_ADDRESS", default = "http://127.0.0.1:8123")]
    pub http_address: String,

    #[envconfig(from = "CLICKHOUSE_TCP_ADDRESS", default = "127.0.0.1:9000")]
    pub tcp_address: String,

    #[envconfig(from = "CLICKHOUSE_USERNAME", default = "default")]
    pub username: String,

    #[envconfig(from = "CLICKHOUSE_PASSWORD", default = "default")]
    pub password: String,

    #[envconfig(from = "CLICKHOUSE_CLUSTER_NAME", default = "meteroid")]
    pub cluster_name: String,

    #[envconfig(from = "CLICKHOUSE_RAW_EVENTS_TABLE", default = "raw_events_v2")]
    pub raw_events_table: String,
    // TODO TLS
}
