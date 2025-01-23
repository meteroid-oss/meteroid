use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub enum OauthProvider {
    #[serde(rename = "google")]
    Google,
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
    pub user_info_url: String,
}

#[derive(Clone)]
pub struct CallbackUrl {
    pub url: SecretString,
    pub csrf_token: SecretString,
    pub pkce_verifier: SecretString,
}
