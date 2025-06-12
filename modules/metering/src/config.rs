use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use envconfig::Envconfig;
use std::net::SocketAddr;

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

    #[envconfig(nested)]
    pub common: CommonConfig,

    #[envconfig(nested)]
    pub internal_auth: InternalAuthConfig,
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
    pub kafka_producer_queue_mib: u32, // Size of the in-memory producer queue in mebibytes

    #[envconfig(from = "KAFKA_MESSAGE_TIMEOUT_MS", default = "20000")]
    pub kafka_message_timeout_ms: u32, // Time before we stop retrying producing a message: 20 seconds

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

        client_config
    }
}

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
    // TODO TLS
}
