use envconfig::Envconfig;

use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use common_config::telemetry::TelemetryConfig;
use kafka::config::KafkaConnectionConfig;
use metering::config::{ClickhouseConfig, Config, KafkaConfig};

pub fn mocked_config(
    meteroid_port: u16,
    metering_port: u16,
    clickhouse_port: u16,
    kafka_port: u16,
    kafka_topic: String,
) -> Config {
    Config {
        clickhouse: ClickhouseConfig {
            database: "meteroid".to_string(),
            address: format!("http://127.0.0.1:{}", clickhouse_port),
            username: "default".to_string(),
            password: "default".to_string(),
        },
        listen_addr: format!("127.0.0.1:{}", metering_port).parse().unwrap(),
        meteroid_endpoint: format!("http://127.0.0.1:{}", meteroid_port),
        common: CommonConfig {
            telemetry: TelemetryConfig::init_from_env().unwrap(),
        },
        internal_auth: InternalAuthConfig {
            hmac_secret: "secret".to_string().into(),
        },
        kafka: KafkaConfig {
            kafka_connection: KafkaConnectionConfig {
                bootstrap_servers: Some(format!("127.0.0.1:{}", kafka_port)),
                security_protocol: None,
                sasl_mechanism: None,
                sasl_username: None,
                sasl_password: None,
            },
            kafka_internal_addr: format!("it_redpanda:{}", 29092),
            kafka_topic,
            kafka_producer_linger_ms: 5,
            kafka_producer_queue_mib: 400,
            kafka_message_timeout_ms: 20000,
            kafka_compression_codec: "none".to_string(),
        },
    }
}
