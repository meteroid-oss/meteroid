use oauth2::TokenResponse;
use oauth2::basic::BasicTokenResponse;
use secrecy::SecretString;
use serde::Deserialize;
use std::time::Duration;
use strum::Display;

#[derive(Debug, Deserialize, Copy, Clone, Display)]
pub enum OauthProvider {
    #[serde(rename = "google")]
    Google,
    #[serde(rename = "hubspot")]
    Hubspot,
    #[serde(rename = "pennylane")]
    Pennylane,
}

#[derive(Debug, Deserialize)]
pub struct OAuthUser {
    pub picture_url: String,
    pub email: String,
    pub sub: String,
}

#[derive(Debug, Deserialize)]
pub struct GoogleUser {
    pub email: String,
    pub email_verified: bool,
    pub picture: String,
    pub sub: String,
}

pub struct OauthProviderConfig {
    pub provider: OauthProvider,
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub callback_url: String,
    pub scopes: Vec<String>,
    pub user_info_url: Option<String>,
}

#[derive(Clone)]
pub struct AuthorizeUrl {
    pub url: SecretString,
    pub csrf_token: SecretString,
    pub pkce_verifier: SecretString,
}

#[derive(Debug, Clone)]
pub struct OAuthTokens {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub expires_in: Option<Duration>,
}

impl From<BasicTokenResponse> for OAuthTokens {
    fn from(response: BasicTokenResponse) -> Self {
        OAuthTokens {
            access_token: SecretString::new(response.access_token().secret().to_owned()),
            refresh_token: response
                .refresh_token()
                .map(|t| SecretString::new(t.secret().to_owned())),
            expires_in: response.expires_in(),
        }
    }
}
