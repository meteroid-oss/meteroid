use std::net::SocketAddr;

use envconfig::Envconfig;
use secrecy::SecretString;

use crate::workers::fang::ext::FangExtConfig;
use common_config::analytics::AnalyticsConfig;
use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use common_config::idempotency::IdempotencyConfig;
use kafka::config::KafkaConnectionConfig;

static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();

#[derive(Envconfig, Debug, Clone)]
pub struct Config {
    #[envconfig(from = "DATABASE_URL")]
    pub database_url: String,

    #[envconfig(from = "METEROID_API_LISTEN_ADDRESS")]
    pub grpc_listen_addr: SocketAddr,

    #[envconfig(from = "METERING_API_EXTERNAL_URL")]
    pub metering_endpoint: String,

    #[envconfig(from = "OBJECT_STORE_URI")]
    pub object_store_uri: String,

    #[envconfig(from = "OBJECT_STORE_PREFIX")]
    pub object_store_prefix: Option<String>,

    #[envconfig(from = "METEROID_REST_API_LISTEN_ADDRESS", default = "127.0.0.1:8080")]
    pub rest_api_addr: SocketAddr,

    #[envconfig(from = "OPENEXCHANGERATES_API_KEY")]
    pub openexchangerates_api_key: Option<String>,

    #[envconfig(nested)]
    pub common: CommonConfig,

    #[envconfig(nested)]
    pub internal_auth: InternalAuthConfig,

    #[envconfig(nested)]
    pub idempotency: IdempotencyConfig,

    #[envconfig(nested)]
    pub analytics: AnalyticsConfig,

    #[envconfig(from = "JWT_SECRET")]
    pub jwt_secret: SecretString,

    #[envconfig(from = "ENABLE_MULTI_ORGANIZATION", default = "false")]
    pub multi_organization_enabled: bool,

    #[envconfig(
        from = "SECRETS_CRYPT_KEY",
        default = "00000000000000000000000000000000"
    )]
    pub secrets_crypt_key: SecretString,

    #[envconfig(nested)]
    pub fang_ext: FangExtConfig,

    #[envconfig(from = "GOTENBERG_URL", default = "http://localhost:3000")]
    pub gotenberg_url: String,

    #[envconfig(from = "SVIX_SERVER_URL")]
    pub svix_server_url: Option<String>,

    #[envconfig(from = "SVIX_JWT_TOKEN")]
    pub svix_jwt_token: SecretString,

    #[envconfig(nested)]
    pub kafka: KafkaConnectionConfig,
}

impl Config {
    pub fn get() -> &'static Self {
        CONFIG.get_or_init(|| Config::init_from_env().unwrap())
    }

    pub fn set(config: Config) -> &'static Self {
        match CONFIG.get() {
            None => {
                CONFIG.set(config).expect("Failed to set config value");
                Config::get()
            }
            Some(v) => {
                panic!("Config value is already set {:?}", v);
            }
        }
    }
}
