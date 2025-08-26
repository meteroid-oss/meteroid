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

#[derive(Debug)]
pub struct OAuthUser {
    pub id: String,
    pub email: String,
    pub company_id: String,
    pub picture_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GoogleOAuthUser {
    pub email: String,
    pub email_verified: bool,
    pub picture: String,
    pub sub: String,
}

impl From<GoogleOAuthUser> for OAuthUser {
    fn from(val: GoogleOAuthUser) -> Self {
        OAuthUser {
            id: val.sub,
            email: val.email,
            company_id: "-".to_string(),
            picture_url: Some(val.picture),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct HubspotOAuthUser {
    pub user_id: u64,
    pub hub_id: u64,
    pub user: String,
}

impl From<HubspotOAuthUser> for OAuthUser {
    fn from(val: HubspotOAuthUser) -> Self {
        OAuthUser {
            id: val.user_id.to_string(),
            email: val.user,
            company_id: val.hub_id.to_string(),
            picture_url: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PennylaneOAuthUser {
    pub user: PennylaneUser,
    pub company: PennylaneCompany,
}

#[derive(Debug, Deserialize)]
pub struct PennylaneUser {
    pub id: u64,
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct PennylaneCompany {
    pub id: u64,
}

impl From<PennylaneOAuthUser> for OAuthUser {
    fn from(val: PennylaneOAuthUser) -> Self {
        OAuthUser {
            id: val.user.id.to_string(),
            email: val.user.email,
            company_id: val.company.id.to_string(),
            picture_url: None,
        }
    }
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
