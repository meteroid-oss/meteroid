use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use envconfig::Envconfig;
use std::net::SocketAddr;

pub use metric_core::config::ClickhouseConfig;

#[cfg(feature = "kafka")]
use kafka::config::KafkaConnectionConfig;
#[cfg(feature = "kafka")]
use rdkafka::ClientConfig;

#[derive(Envconfig, Clone)]
pub struct Config {
    #[envconfig(from = "METERING_API_LISTEN_ADDRESS", default = "127.0.0.1:8080")]
    pub listen_addr: SocketAddr,

    #[envconfig(from = "METEROID_API_EXTERNAL_URL", default = "http://127.0.0.1:50061")]
    pub meteroid_endpoint: String,

    #[cfg(feature = "kafka")]
    #[envconfig(nested)]
    pub kafka: KafkaConfig,

    #[cfg(feature = "clickhouse")]
    #[envconfig(nested)]
    pub clickhouse: ClickhouseConfig,

    #[cfg(feature = "clickhouse")]
    #[envconfig(nested)]
    pub consumer: ConsumerConfig,

    #[envconfig(nested)]
    pub common: CommonConfig,

    #[envconfig(nested)]
    pub internal_auth: InternalAuthConfig,
}

#[cfg(feature = "clickhouse")]
#[derive(Envconfig, Clone, Debug)]
pub struct ConsumerConfig {
    /// `auto` is a majority of replicas. `1` resumes writes during a replica outage on two
    /// replicas, at the risk of duplicates (ADR-0011).
    #[envconfig(from = "CLICKHOUSE_CONSUMER_INSERT_QUORUM", default = "auto")]
    pub insert_quorum: String,

    #[envconfig(
        from = "CLICKHOUSE_CONSUMER_INSERT_QUORUM_TIMEOUT_MS",
        default = "30000"
    )]
    pub insert_quorum_timeout_ms: u64,

    /// How long a failed insert is retried with the same batch before the consumer restarts.
    #[envconfig(
        from = "CLICKHOUSE_CONSUMER_INSERT_RETRY_BUDGET_MS",
        default = "240000"
    )]
    pub insert_retry_budget_ms: u64,

    #[envconfig(
        from = "CLICKHOUSE_RAW_EVENTS_LOCAL_TABLE",
        default = "raw_events_local_v2"
    )]
    pub raw_events_local_table: String,

    #[envconfig(from = "CLICKHOUSE_EVENT_IDS_LOCAL_TABLE", default = "event_ids_local")]
    pub event_ids_local_table: String,
}

#[cfg(feature = "kafka")]
#[derive(Envconfig, Clone, Debug)]
pub struct KafkaConfig {
    // TODO if using clickhouse kafka table engine with auth or schema, we need to pass the auth data through clickhouse server xml config as well
    #[envconfig(nested)]
    pub kafka_connection: KafkaConnectionConfig,

    // used by clickhouse kafka table engine
    #[envconfig(from = "KAFKA_INTERNAL_ADDR", default = "redpanda:29092")]
    pub kafka_internal_addr: String,

    #[envconfig(from = "KAFKA_RAW_TOPIC", default = "meteroid-events-raw")]
    pub kafka_raw_topic: String,

    #[envconfig(from = "KAFKA_PRODUCER_LINGER_MS", default = "20")]
    pub kafka_producer_linger_ms: u32, // Maximum time between producer batches during low traffic

    #[envconfig(from = "KAFKA_PRODUCER_QUEUE_MIB", default = "400")]
    pub kafka_producer_queue_mib: u32,

    #[envconfig(from = "KAFKA_MESSAGE_TIMEOUT_MS", default = "20000")]
    pub kafka_message_timeout_ms: u32,

    #[envconfig(from = "KAFKA_COMPRESSION_CODEC", default = "none")]
    pub kafka_compression_codec: String, // none, gzip, snappy, lz4, zstd
}

#[cfg(feature = "kafka")]
impl KafkaConfig {
    pub fn to_client_config(&self) -> ClientConfig {
        let mut client_config = self.kafka_connection.to_client_config();

        client_config.set("linger.ms", self.kafka_producer_linger_ms.to_string());
        client_config.set(
            "message.timeout.ms",
            self.kafka_message_timeout_ms.to_string(),
        );
        client_config.set("compression.codec", self.kafka_compression_codec.clone());
        client_config.set(
            "queue.buffering.max.kbytes",
            (self.kafka_producer_queue_mib * 1024).to_string(),
        );
        client_config.set("enable.idempotence", "true");

        client_config
    }
}
