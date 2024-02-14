use std::net::SocketAddr;

use crate::workers::fang::ext::FangExtConfig;
use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use common_config::idempotency::IdempotencyConfig;
use envconfig::Envconfig;
use secrecy::SecretString;
use common_config::tracking::TrackingConfig;

static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();

#[derive(Envconfig, Debug, Clone)]
pub struct Config {
    #[envconfig(from = "DATABASE_URL")]
    pub database_url: String,

    #[envconfig(from = "METEROID_API_LISTEN_ADDRESS")]
    pub listen_addr: SocketAddr,

    #[envconfig(from = "METERING_API_EXTERNAL_URL")]
    pub metering_endpoint: String,

    #[envconfig(from = "OBJECT_STORE_URI")]
    pub object_store_uri: String,

    #[envconfig(from = "INVOICING_WEBHOOK_LISTEN_ADDRESS")]
    pub invoicing_webhook_addr: SocketAddr,

    #[envconfig(nested = true)]
    pub common: CommonConfig,

    #[envconfig(nested = true)]
    pub internal_auth: InternalAuthConfig,

    #[envconfig(nested = true)]
    pub idempotency: IdempotencyConfig,

    #[envconfig(nested = true)]
    pub tracking: TrackingConfig,

    #[envconfig(from = "JWT_SECRET")]
    pub jwt_secret: SecretString,

    #[envconfig(
        from = "SECRETS_CRYPT_KEY",
        default = "00000000000000000000000000000000"
    )]
    pub secrets_crypt_key: SecretString,

    #[envconfig(nested = true)]
    pub fang_ext: FangExtConfig,
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
