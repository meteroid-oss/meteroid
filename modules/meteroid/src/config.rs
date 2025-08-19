use envconfig::Envconfig;
use secrecy::SecretString;
use std::net::SocketAddr;
use std::str::FromStr;

use crate::workers::fang::ext::FangExtConfig;
use common_config::analytics::AnalyticsConfig;
use common_config::auth::InternalAuthConfig;
use common_config::common::CommonConfig;
use common_config::idempotency::IdempotencyConfig;
use meteroid_mailer::config::MailerConfig;
use meteroid_oauth::config::OauthConfig;

static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();

#[derive(Envconfig, Debug, Clone)]
pub struct Config {
    #[envconfig(from = "DATABASE_URL")]
    pub database_url: String,

    #[envconfig(from = "METEROID_PUBLIC_URL", default = "https://meteroid.com")]
    pub public_url: String,

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

    #[envconfig(
        from = "METEROID_REST_API_EXTERNAL_URL",
        default = "http://127.0.0.1:8080"
    )]
    pub rest_api_external_url: String,

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
    pub secrets_crypt_key: CryptKey,

    #[envconfig(nested)]
    pub fang_ext: FangExtConfig,

    #[envconfig(from = "SVIX_SERVER_URL")]
    pub svix_server_url: Option<String>,

    #[envconfig(from = "SVIX_JWT_TOKEN")]
    pub svix_jwt_token: SecretString,

    #[envconfig(nested)]
    pub mailer: MailerConfig,

    #[envconfig(nested)]
    pub oauth: OauthConfig,

    #[envconfig(from = "DOMAINS_WHITELIST")]
    pub domains_whitelist: Option<DomainWhitelist>,
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

    pub fn mailer_enabled(&self) -> bool {
        self.mailer
            .smtp_host
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    pub fn domains_whitelist(&self) -> Vec<String> {
        match &self.domains_whitelist {
            Some(DomainWhitelist(domains)) => domains.clone(),
            None => vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct DomainWhitelist(pub Vec<String>);
impl FromStr for DomainWhitelist {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let domains: Vec<String> = s
            .split(',')
            .map(|d| d.trim().to_lowercase().to_string())
            .collect();
        if domains.is_empty() {
            return Err("Domain whitelist cannot be empty".to_string());
        }
        Ok(DomainWhitelist(domains))
    }
}

#[derive(Debug, Clone)]
pub struct CryptKey(pub SecretString);
impl FromStr for CryptKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 32 {
            return Err("Crypt key must be exactly 32 characters long".to_string());
        }
        Ok(CryptKey(SecretString::new(s.to_string())))
    }
}
