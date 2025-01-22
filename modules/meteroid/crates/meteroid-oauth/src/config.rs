use envconfig::Envconfig;
use secrecy::SecretString;

#[derive(Envconfig, Debug, Clone)]
pub struct OauthConfig {
    #[envconfig(from = "METEROID_PUBLIC_URL", default = "https://meteroid.com")]
    pub public_url: String,
    #[envconfig(nested)]
    pub google: GoogleOauthConfig,
}

impl OauthConfig {
    pub fn dummy() -> Self {
        Self {
            public_url: "https://meteroid.com".to_owned(),
            google: GoogleOauthConfig {
                client_id: SecretString::new("google_client_id".to_owned()),
                client_secret: SecretString::new("google_client_secret".to_owned()),
            },
        }
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct GoogleOauthConfig {
    #[envconfig(from = "OAUTH_GOOGLE_CLIENT_ID")]
    pub client_id: SecretString,
    #[envconfig(from = "OAUTH_GOOGLE_CLIENT_SECRET")]
    pub client_secret: SecretString,
}
