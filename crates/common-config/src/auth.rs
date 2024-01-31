use envconfig::Envconfig;
use secrecy::SecretString;

#[derive(Envconfig, Debug, Clone)]
pub struct InternalAuthConfig {
    #[envconfig(from = "INTERNAL_API_SECRET")]
    pub hmac_secret: SecretString,
}
