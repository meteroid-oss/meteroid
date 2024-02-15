use envconfig::Envconfig;
use secrecy::SecretString;

#[derive(Envconfig, Debug, Clone)]
pub struct AnalyticsConfig {
    #[envconfig(from = "ANALYTICS_ENABLED", default = "true")]
    pub enabled: bool,

    #[envconfig(
        from = "ANALYTICS_API_KEY",
        default = "4YKpMUjndDYFqjJ1e85gEA4vbm7DIO6p"
    )]
    pub api_key: SecretString,
}
