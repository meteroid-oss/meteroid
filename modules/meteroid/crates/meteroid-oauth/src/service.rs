/// Inspired by https://github.com/mpiorowski/rusve/service-auth
use crate::errors::OauthServiceError;
use std::fmt::{Display, Formatter};

use crate::model::{AuthorizeUrl, OAuthTokens, OAuthUser, OauthProvider};
use async_trait::async_trait;
use error_stack::{ResultExt, bail};
use oauth2::basic::{
    BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
    BasicTokenResponse,
};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken,
    EndpointNotSet, EndpointSet, ErrorResponse, PkceCodeChallenge, PkceCodeVerifier, RefreshToken,
    Scope, StandardRevocableToken, TokenUrl,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type BasicClient<
    BER = BasicErrorResponse,
    HasAuthUrl = EndpointNotSet,
    HasDeviceAuthUrl = EndpointNotSet,
    HasIntrospectionUrl = EndpointNotSet,
    HasRevocationUrl = EndpointNotSet,
    HasTokenUrl = EndpointNotSet,
> = Client<
    BER,
    BasicTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
>;

type Oauth2BasicClient<BER> =
    BasicClient<BER, EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

#[async_trait]
pub trait OauthService: Send + Sync {
    fn client_id(&self) -> String;

    fn authorize_url(&self) -> AuthorizeUrl;

    async fn exchange_code(
        &self,
        auth_code: SecretString,
        pkce_code_verifier: SecretString,
    ) -> error_stack::Result<OAuthTokens, OauthServiceError>;

    async fn exchange_refresh_token(
        &self,
        refresh_token: SecretString,
    ) -> error_stack::Result<OAuthTokens, OauthServiceError>;

    async fn get_user_info(
        &self,
        access_token: SecretString,
    ) -> error_stack::Result<OAuthUser, OauthServiceError>;
}

#[derive(Clone)]
pub struct OauthServices {
    google: Option<Arc<dyn OauthService>>,
    hubspot: Option<Arc<dyn OauthService>>,
    pennylane: Option<Arc<dyn OauthService>>,
}

impl OauthServices {
    pub fn new(config: crate::config::OauthConfig) -> Self {
        Self {
            google: Self::google(&config),
            hubspot: Self::hubspot(&config),
            pennylane: Self::pennylane(&config),
        }
    }

    pub fn for_provider(&self, provider: OauthProvider) -> Option<Arc<dyn OauthService>> {
        match provider {
            OauthProvider::Google => self.google.clone(),
            OauthProvider::Hubspot => self.hubspot.clone(),
            OauthProvider::Pennylane => self.pennylane.clone(),
        }
    }

    pub fn client_id(&self, provider: OauthProvider) -> Option<String> {
        self.for_provider(provider)
            .map(|provider| provider.client_id())
    }

    fn google(config: &crate::config::OauthConfig) -> Option<Arc<dyn OauthService>> {
        if let (Some(client_id), Some(client_secret)) = (
            config.google.client_id.as_ref(),
            config.google.client_secret.as_ref(),
        ) {
            let cfg = crate::model::OauthProviderConfig {
                provider: OauthProvider::Google,
                client_id: client_id.expose_secret().to_owned(),
                client_secret: client_secret.expose_secret().to_owned(),
                auth_url: "https://accounts.google.com/o/oauth2/auth".to_owned(),
                token_url: "https://www.googleapis.com/oauth2/v3/token".to_string(),
                callback_url: format!(
                    "{}/oauth-callback/google",
                    config.rest_api_external_url.as_str()
                ),
                user_info_url: Some("https://www.googleapis.com/oauth2/v3/userinfo".to_string()),
                scopes: vec!["email".to_string(), "openid".to_string()],
            };
            Some(Arc::new(OauthServiceImpl::<BasicErrorResponse> {
                oauth_client: OauthServiceImpl::oauth_basic_client(&cfg),
                http_client: http_client(),
                config: cfg,
            }))
        } else {
            None
        }
    }

    fn hubspot(config: &crate::config::OauthConfig) -> Option<Arc<dyn OauthService>> {
        if let (Some(client_id), Some(client_secret)) = (
            config.hubspot.client_id.as_ref(),
            config.hubspot.client_secret.as_ref(),
        ) {
            let cfg = crate::model::OauthProviderConfig {
                provider: OauthProvider::Hubspot,
                client_id: client_id.expose_secret().to_owned(),
                client_secret: client_secret.expose_secret().to_owned(),
                auth_url: "https://app.hubspot.com/oauth/authorize".to_owned(),
                token_url: "https://api.hubapi.com/oauth/v1/token".to_string(),
                callback_url: format!(
                    "{}/oauth-callback/hubspot",
                    config.rest_api_external_url.as_str()
                ),
                user_info_url: None,
                scopes: vec![
                    "oauth".to_owned(),
                    "crm.objects.deals.read".to_owned(),
                    "crm.objects.deals.write".to_owned(),
                    "crm.schemas.deals.read".to_owned(),
                    "crm.schemas.deals.write".to_owned(),
                    "crm.objects.contacts.read".to_owned(),
                    "crm.objects.contacts.write".to_owned(),
                    "crm.schemas.contacts.read".to_owned(),
                    "crm.schemas.contacts.write".to_owned(),
                    "crm.objects.companies.read".to_owned(),
                    "crm.objects.companies.write".to_owned(),
                    "crm.schemas.companies.read".to_owned(),
                    "crm.schemas.companies.write".to_owned(),
                ],
            };
            Some(Arc::new(OauthServiceImpl::<HubspotErrorResponse> {
                oauth_client: OauthServiceImpl::oauth_basic_client(&cfg),
                http_client: http_client(),
                config: cfg,
            }))
        } else {
            None
        }
    }

    fn pennylane(config: &crate::config::OauthConfig) -> Option<Arc<dyn OauthService>> {
        if let (Some(client_id), Some(client_secret)) = (
            config.pennylane.client_id.as_ref(),
            config.pennylane.client_secret.as_ref(),
        ) {
            let cfg = crate::model::OauthProviderConfig {
                provider: OauthProvider::Pennylane,
                client_id: client_id.expose_secret().to_owned(),
                client_secret: client_secret.expose_secret().to_owned(),
                auth_url: "https://app.pennylane.com/oauth/authorize".to_owned(),
                token_url: "https://app.pennylane.com/oauth/token".to_string(),
                // todo migrate to rest api url once pennylane app is updated
                callback_url: format!("{}/oauth-callback/pennylane", config.public_url.as_str()),
                user_info_url: None,
                scopes: vec![
                    "customers:all".to_owned(),
                    "customer_invoices:all".to_owned(),
                    "file_attachments:all".to_owned(),
                ],
            };
            Some(Arc::new(OauthServiceImpl::<PennylaneErrorResponse> {
                oauth_client: OauthServiceImpl::oauth_basic_client(&cfg),
                http_client: http_client(),
                config: cfg,
            }))
        } else {
            None
        }
    }
}

struct OauthServiceImpl<TER: ErrorResponse> {
    config: crate::model::OauthProviderConfig,
    http_client: reqwest::Client,
    oauth_client: Oauth2BasicClient<TER>,
}

impl<BER: ErrorResponse + 'static> OauthServiceImpl<BER> {
    fn oauth_basic_client(config: &crate::model::OauthProviderConfig) -> Oauth2BasicClient<BER> {
        let auth_url =
            AuthUrl::new(config.auth_url.to_owned()).expect("Invalid authorization endpoint URL");
        let token_url =
            TokenUrl::new(config.token_url.to_owned()).expect("Invalid token endpoint URL");
        let redirect_url =
            oauth2::RedirectUrl::new(config.callback_url.to_owned()).expect("Invalid redirect URL");

        BasicClient::new(ClientId::new(config.client_id.to_owned()))
            .set_client_secret(ClientSecret::new(config.client_secret.to_owned()))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect_url)
            .set_auth_type(AuthType::RequestBody)
    }
}

#[async_trait]
impl<T: ErrorResponse + Send + Sync + 'static> OauthService for OauthServiceImpl<T> {
    fn client_id(&self) -> String {
        self.config.client_id.to_owned()
    }

    fn authorize_url(&self) -> AuthorizeUrl {
        // Generate a PKCE challenge.
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        // Generate the full authorization URL.
        let client = &self.oauth_client;
        let mut client = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);

        for scope in &self.config.scopes {
            client = client.add_scope(Scope::new(scope.to_owned()));
        }

        let (auth_url, csrf_token) = client.add_extra_param("access_type", "offline").url();

        AuthorizeUrl {
            url: SecretString::new(auth_url.to_string()),
            csrf_token: SecretString::new(csrf_token.secret().to_owned()),
            pkce_verifier: SecretString::new(pkce_verifier.secret().to_owned()),
        }
    }

    async fn exchange_code(
        &self,
        auth_code: SecretString,
        pkce_code_verifier: SecretString,
    ) -> error_stack::Result<OAuthTokens, OauthServiceError> {
        let client = &self.oauth_client;

        client
            .exchange_code(AuthorizationCode::new(auth_code.expose_secret().to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(
                pkce_code_verifier.expose_secret().to_owned(),
            ))
            .request_async(&self.http_client)
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange code".into(),
            ))
            .map(From::from)
    }

    async fn exchange_refresh_token(
        &self,
        refresh_token: SecretString,
    ) -> error_stack::Result<OAuthTokens, OauthServiceError> {
        let client = &self.oauth_client;

        client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.expose_secret().to_owned()))
            .request_async(&self.http_client)
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange refresh_token".into(),
            ))
            .map(From::from)
    }

    async fn get_user_info(
        &self,
        access_token: SecretString,
    ) -> error_stack::Result<OAuthUser, OauthServiceError> {
        match self.config.provider {
            OauthProvider::Google => {
                let url = self
                    .config
                    .user_info_url
                    .as_ref()
                    .ok_or(OauthServiceError::UserInfoNotSupported)?;

                let user_profile = self
                    .http_client
                    .get(url)
                    .header(
                        reqwest::header::AUTHORIZATION,
                        format!("Bearer {}", access_token.expose_secret()),
                    )
                    .send()
                    .await
                    .change_context(OauthServiceError::ProviderApi(
                        "Failed to fetch user info".into(),
                    ))?;

                let user_profile = user_profile
                    .json::<crate::model::GoogleUser>()
                    .await
                    .change_context(OauthServiceError::ProviderApi(
                        "Failed to deserialize user info response".into(),
                    ))?;

                if !user_profile.email_verified {
                    bail!(OauthServiceError::UserEmailNotVerified);
                }

                Ok(OAuthUser {
                    picture_url: user_profile.picture,
                    email: user_profile.email,
                    sub: user_profile.sub,
                })
            }
            OauthProvider::Hubspot => {
                bail!(OauthServiceError::UserInfoNotSupported)
            }
            OauthProvider::Pennylane => {
                bail!(OauthServiceError::UserInfoNotSupported)
            }
        }
    }
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::ClientBuilder::new()
        // Following redirects opens the client up to SSRF vulnerabilities.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Client should build")
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct HubspotErrorResponse {
    pub status: String,
    pub message: String,
    #[serde(rename = "correlationId")]
    pub correlation_id: String,
}

impl Display for HubspotErrorResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", &self)
    }
}

impl ErrorResponse for HubspotErrorResponse {}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PennylaneErrorResponse {
    pub error: Option<String>,
    pub error_description: Option<String>,
}

impl Display for PennylaneErrorResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", &self)
    }
}

impl ErrorResponse for PennylaneErrorResponse {}

#[cfg(test)]
mod tests {
    use crate::config::OauthConfig;
    use crate::model::OauthProvider;
    use crate::service::OauthServices;
    use secrecy::{ExposeSecret, SecretString};
    use std::str::FromStr;

    #[tokio::test]
    async fn test_authorize_url() -> Result<(), Box<dyn std::error::Error>> {
        let srv = OauthServices::new(OauthConfig {
            rest_api_external_url: "http://localhost:8080".to_string(),
            public_url: "https://meteroid.com".to_string(),
            google: crate::config::GoogleOauthConfig {
                client_id: Some(SecretString::from_str("client_id").unwrap()),
                client_secret: Some(SecretString::from_str("client_secret").unwrap()),
            },
            hubspot: crate::config::HubspotOauthConfig {
                client_id: Some(SecretString::from_str("client_id").unwrap()),
                client_secret: Some(SecretString::from_str("client_secret").unwrap()),
            },
            pennylane: crate::config::PennylaneOauthConfig {
                client_id: Some(SecretString::from_str("client_id").unwrap()),
                client_secret: Some(SecretString::from_str("client_secret").unwrap()),
            },
        });

        let url = srv
            .for_provider(OauthProvider::Google)
            .unwrap()
            .authorize_url();

        let url = url.url.expose_secret();

        assert!(url.starts_with("https://accounts.google.com/o/oauth2/auth?response_type=code&client_id=client_id&state="));
        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Foauth-callback%2Fgoogle")
        );
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("scope=email+openid"));

        Ok(())
    }
}
