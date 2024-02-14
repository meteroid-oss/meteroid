use envconfig::Envconfig;
use secrecy::SecretString;

#[derive(Envconfig, Debug, Clone)]
pub struct TrackingConfig {
    #[envconfig(from = "TRACKING_ENABLED", default = "true")]
    pub enabled: bool,

    #[envconfig(from = "TRACKING_API_KEY")]
    pub api_key: SecretString,
}
