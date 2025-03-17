use envconfig::Envconfig;
use secrecy::SecretString;

#[derive(Envconfig, Debug, Clone)]
pub struct OauthConfig {
    #[envconfig(from = "METEROID_PUBLIC_URL", default = "https://meteroid.com")]
    pub public_url: String,
    #[envconfig(nested)]
    pub google: GoogleOauthConfig,
    #[envconfig(nested)]
    pub hubspot: HubspotOauthConfig,
}

impl OauthConfig {
    pub fn dummy() -> Self {
        Self {
            public_url: "https://meteroid.com".to_owned(),
            google: GoogleOauthConfig {
                client_id: Some(SecretString::new("google_client_id".to_owned())),
                client_secret: Some(SecretString::new("google_client_secret".to_owned())),
            },
            hubspot: HubspotOauthConfig {
                client_id: Some(SecretString::new("hubspot_client_id".to_owned())),
                client_secret: Some(SecretString::new("hubspot_client_secret".to_owned())),
            },
        }
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct GoogleOauthConfig {
    #[envconfig(from = "OAUTH_GOOGLE_CLIENT_ID")]
    pub client_id: Option<SecretString>,
    #[envconfig(from = "OAUTH_GOOGLE_CLIENT_SECRET")]
    pub client_secret: Option<SecretString>,
}

impl GoogleOauthConfig {
    pub fn is_enabled(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }
}

#[derive(Envconfig, Debug, Clone)]
pub struct HubspotOauthConfig {
    #[envconfig(from = "OAUTH_HUBSPOT_CLIENT_ID")]
    pub client_id: Option<SecretString>,
    #[envconfig(from = "OAUTH_HUBSPOT_CLIENT_SECRET")]
    pub client_secret: Option<SecretString>,
}
