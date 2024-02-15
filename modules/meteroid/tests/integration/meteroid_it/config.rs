use std::net::SocketAddr;

use envconfig::Envconfig;

use common_config::analytics::AnalyticsConfig;
use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use common_config::idempotency::IdempotencyConfig;
use common_config::telemetry::TelemetryConfig;
use meteroid::config::Config;
use meteroid::workers::fang::ext::FangExtConfig;

pub fn mocked_config(
    postgres_connection_string: String,
    invoicing_webhook_addr: SocketAddr,
    meteroid_port: u16,
    metering_port: u16,
) -> Config {
    Config {
        database_url: postgres_connection_string.to_owned(),
        listen_addr: format!("127.0.0.1:{}", meteroid_port).parse().unwrap(),
        metering_endpoint: format!("http://127.0.0.1:{}", metering_port)
            .parse()
            .unwrap(),
        object_store_uri: "".to_owned(),
        invoicing_webhook_addr,
        common: CommonConfig {
            telemetry: TelemetryConfig::init_from_env().unwrap(),
        },
        internal_auth: InternalAuthConfig {
            hmac_secret: "secret".to_string().into(),
        },
        idempotency: IdempotencyConfig {
            required: false,
            ttl: "5s".parse().unwrap(),
            size: 100000,
        },
        analytics: AnalyticsConfig {
            enabled: false,
            api_key: "".to_string().into(),
        },
        jwt_secret: "secret".to_string().into(),
        secrets_crypt_key: "00000000000000000000000000000000".to_string().into(),
        fang_ext: FangExtConfig::init_from_env().unwrap(),
    }
}
