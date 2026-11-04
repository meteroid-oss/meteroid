/// Inspired by <https://github.com/mpiorowski/rusve/service-auth>
use crate::errors::OauthServiceError;
use std::fmt::{Display, Formatter};

use crate::model::{AuthorizeUrl, OAuthTokens, OAuthUser, OauthProvider};
use async_trait::async_trait;
use egress_http::{EgressClient, Url};
use error_stack::{Report, ResultExt, bail};
use oauth2::basic::{
    BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
    BasicTokenResponse,
};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken,
    EndpointNotSet, EndpointSet, ErrorResponse, PkceCodeChallenge, PkceCodeVerifier, RefreshToken,
    Scope, StandardRevocableToken, TokenUrl,
};
use oauth2_reqwest::ReqwestClient;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

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
    ) -> Result<OAuthTokens, Report<OauthServiceError>>;

    async fn exchange_refresh_token(
        &self,
        refresh_token: SecretString,
    ) -> Result<OAuthTokens, Report<OauthServiceError>>;

    async fn get_user_info(
        &self,
        access_token: SecretString,
    ) -> Result<OAuthUser, Report<OauthServiceError>>;
}

#[derive(Clone)]
pub struct OauthServices {
    google: Option<Arc<dyn OauthService>>,
    hubspot: Option<Arc<dyn OauthService>>,
    pennylane: Option<Arc<dyn OauthService>>,
    egress: EgressClient,
}

impl OauthServices {
    /// `egress` guards the per-organization SSO endpoints, which are admin-supplied
    /// and therefore untrusted; the built-in providers above use fixed hosts.
    pub fn new(config: crate::config::OauthConfig, egress: EgressClient) -> Self {
        Self {
            google: Self::google(&config),
            hubspot: Self::hubspot(&config),
            pennylane: Self::pennylane(&config),
            egress,
        }
    }

    pub fn egress(&self) -> &EgressClient {
        &self.egress
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
                user_info_url: Some("https://api.hubapi.com/oauth/v1/access-tokens".to_string()),
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
                callback_url: format!(
                    "{}/oauth-callback/pennylane",
                    config.rest_api_external_url.as_str()
                ),
                user_info_url: Some("https://app.pennylane.com/api/external/v2/me".to_string()),
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
            AuthUrl::new(config.auth_url.clone()).expect("Invalid authorization endpoint URL");
        let token_url =
            TokenUrl::new(config.token_url.clone()).expect("Invalid token endpoint URL");
        let redirect_url =
            oauth2::RedirectUrl::new(config.callback_url.clone()).expect("Invalid redirect URL");

        BasicClient::new(ClientId::new(config.client_id.clone()))
            .set_client_secret(ClientSecret::new(config.client_secret.clone()))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect_url)
            .set_auth_type(AuthType::RequestBody)
    }
}

#[async_trait]
impl<T: ErrorResponse + Send + Sync + 'static> OauthService for OauthServiceImpl<T> {
    fn client_id(&self) -> String {
        self.config.client_id.clone()
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
            url: SecretString::from(auth_url.to_string()),
            csrf_token: SecretString::from(csrf_token.secret().to_owned()),
            pkce_verifier: SecretString::from(pkce_verifier.secret().to_owned()),
        }
    }

    async fn exchange_code(
        &self,
        auth_code: SecretString,
        pkce_code_verifier: SecretString,
    ) -> Result<OAuthTokens, Report<OauthServiceError>> {
        let client = &self.oauth_client;

        client
            .exchange_code(AuthorizationCode::new(auth_code.expose_secret().to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(
                pkce_code_verifier.expose_secret().to_owned(),
            ))
            .request_async(&ReqwestClient::from(self.http_client.clone()))
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange code".into(),
            ))
            .map(From::from)
    }

    async fn exchange_refresh_token(
        &self,
        refresh_token: SecretString,
    ) -> Result<OAuthTokens, Report<OauthServiceError>> {
        let client = &self.oauth_client;

        client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.expose_secret().to_owned()))
            .request_async(&ReqwestClient::from(self.http_client.clone()))
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange refresh_token".into(),
            ))
            .map(From::from)
    }

    async fn get_user_info(
        &self,
        access_token: SecretString,
    ) -> Result<OAuthUser, Report<OauthServiceError>> {
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
                    .json::<crate::model::GoogleOAuthUser>()
                    .await
                    .change_context(OauthServiceError::ProviderApi(
                        "Failed to deserialize user info response".into(),
                    ))?;

                if !user_profile.email_verified {
                    bail!(OauthServiceError::UserEmailNotVerified);
                }

                Ok(user_profile.into())
            }
            OauthProvider::Hubspot => {
                let url = self
                    .config
                    .user_info_url
                    .as_ref()
                    .map(|url| format!("{url}/{}", access_token.expose_secret()))
                    .ok_or(OauthServiceError::UserInfoNotSupported)?;

                let user_profile = self.http_client.get(url).send().await.change_context(
                    OauthServiceError::ProviderApi("Failed to fetch user info".into()),
                )?;

                let user_profile = user_profile
                    .json::<crate::model::HubspotOAuthUser>()
                    .await
                    .change_context(OauthServiceError::ProviderApi(
                        "Failed to deserialize user info response".into(),
                    ))?;

                Ok(user_profile.into())
            }
            OauthProvider::Pennylane => {
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
                    .json::<crate::model::PennylaneOAuthUser>()
                    .await
                    .change_context(OauthServiceError::ProviderApi(
                        "Failed to deserialize user info response".into(),
                    ))?;

                Ok(user_profile.into())
            }
        }
    }
}

pub(crate) fn http_client() -> reqwest::Client {
    http_client_tracing::client_builder()
        // Following redirects opens the client up to SSRF vulnerabilities.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("Client should build")
}

/// Fetch and parse an IdP's OIDC discovery document, so an admin only has to
/// provide the issuer URL instead of copying every endpoint by hand.
pub async fn discover_oidc(
    egress: &EgressClient,
    issuer_url: &str,
) -> Result<crate::model::OidcDiscoveryDocument, Report<OauthServiceError>> {
    let well_known = well_known_url(egress, issuer_url)?;

    let resp = egress
        .reqwest()
        .get(well_known)
        .send()
        .await
        .change_context(OauthServiceError::ProviderApi(
            "failed to fetch the OIDC discovery document".into(),
        ))?;

    if !resp.status().is_success() {
        bail!(OauthServiceError::ProviderApi(format!(
            "the OIDC discovery document request returned HTTP {}",
            resp.status().as_u16()
        )));
    }

    resp.json::<crate::model::OidcDiscoveryDocument>()
        .await
        .change_context(OauthServiceError::ProviderApi(
            "failed to parse the OIDC discovery document".into(),
        ))
}

/// Build the `.well-known/openid-configuration` URL from an issuer, refusing any
/// destination the egress rules reject — the discovery GET is server-side, so an
/// unchecked issuer is a port scanner pointed at the cluster.
pub(crate) fn well_known_url(
    egress: &EgressClient,
    issuer_url: &str,
) -> Result<Url, Report<OauthServiceError>> {
    let issuer = issuer_url.trim().trim_end_matches('/');
    let well_known = if issuer.ends_with("/.well-known/openid-configuration") {
        issuer.to_string()
    } else {
        format!("{issuer}/.well-known/openid-configuration")
    };
    validate_endpoint(egress, "issuer", &well_known)
}

/// Parse and check one admin-supplied OIDC endpoint against the egress rules.
pub fn validate_endpoint(
    egress: &EgressClient,
    name: &str,
    url: &str,
) -> Result<Url, Report<OauthServiceError>> {
    let parsed = Url::parse(url.trim()).change_context(OauthServiceError::ProviderApi(format!(
        "the {name} URL is not a valid URL"
    )))?;
    if let Err(e) = egress.validate_url(&parsed) {
        bail!(OauthServiceError::ProviderApi(format!(
            "the {name} URL is not an allowed destination: {e}"
        )));
    }
    Ok(parsed)
}

/// Build an `OauthService` for a per-organization OIDC SSO connection.
/// Endpoints come from admin-provided data, so URL parsing is fallible here —
/// unlike the server-config providers, which `expect()` valid URLs at startup.
pub fn build_oidc_sso_service(
    egress: &EgressClient,
    cfg: crate::model::OidcSsoConfig,
) -> Result<Arc<dyn OauthService>, Report<OauthServiceError>> {
    // Re-checked here rather than only at upsert: a row may predate the check, and
    // the token/userinfo fetches fire on the unauthenticated sign-in path.
    validate_endpoint(egress, "authorize", &cfg.authorize_url)?;
    validate_endpoint(egress, "token", &cfg.token_url)?;
    validate_endpoint(egress, "userinfo", &cfg.userinfo_url)?;

    let oauth_client = build_basic_client(
        &cfg.client_id,
        &cfg.client_secret,
        &cfg.authorize_url,
        &cfg.token_url,
        &cfg.callback_url,
    )?;
    Ok(Arc::new(OidcSsoService {
        http_client: egress.reqwest().clone(),
        oauth_client,
        client_id: cfg.client_id,
        userinfo_url: cfg.userinfo_url,
        scopes: cfg.scopes,
    }))
}

fn build_basic_client(
    client_id: &str,
    client_secret: &str,
    auth_url: &str,
    token_url: &str,
    redirect_url: &str,
) -> Result<Oauth2BasicClient<BasicErrorResponse>, Report<OauthServiceError>> {
    let auth = AuthUrl::new(auth_url.to_string()).change_context(
        OauthServiceError::ProviderApi("invalid authorize url".into()),
    )?;
    let token = TokenUrl::new(token_url.to_string())
        .change_context(OauthServiceError::ProviderApi("invalid token url".into()))?;
    let redirect = oauth2::RedirectUrl::new(redirect_url.to_string()).change_context(
        OauthServiceError::ProviderApi("invalid redirect url".into()),
    )?;

    Ok(BasicClient::new(ClientId::new(client_id.to_string()))
        .set_client_secret(ClientSecret::new(client_secret.to_string()))
        .set_auth_uri(auth)
        .set_token_uri(token)
        .set_redirect_uri(redirect)
        .set_auth_type(AuthType::RequestBody))
}

/// OIDC SSO service. The oauth2 mechanics (authorize URL, PKCE, code exchange)
/// mirror `OauthServiceImpl`; `get_user_info` speaks generic OIDC `userinfo`
/// so any compliant IdP works without a per-provider arm.
struct OidcSsoService {
    http_client: reqwest::Client,
    oauth_client: Oauth2BasicClient<BasicErrorResponse>,
    client_id: String,
    userinfo_url: String,
    scopes: Vec<String>,
}

#[async_trait]
impl OauthService for OidcSsoService {
    fn client_id(&self) -> String {
        self.client_id.clone()
    }

    fn authorize_url(&self) -> AuthorizeUrl {
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        let mut client = self
            .oauth_client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);

        for scope in &self.scopes {
            client = client.add_scope(Scope::new(scope.to_owned()));
        }

        let (auth_url, csrf_token) = client.url();

        AuthorizeUrl {
            url: SecretString::from(auth_url.to_string()),
            csrf_token: SecretString::from(csrf_token.secret().to_owned()),
            pkce_verifier: SecretString::from(pkce_verifier.secret().to_owned()),
        }
    }

    async fn exchange_code(
        &self,
        auth_code: SecretString,
        pkce_code_verifier: SecretString,
    ) -> Result<OAuthTokens, Report<OauthServiceError>> {
        self.oauth_client
            .exchange_code(AuthorizationCode::new(auth_code.expose_secret().to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(
                pkce_code_verifier.expose_secret().to_owned(),
            ))
            .request_async(&ReqwestClient::from(self.http_client.clone()))
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange code".into(),
            ))
            .map(From::from)
    }

    async fn exchange_refresh_token(
        &self,
        refresh_token: SecretString,
    ) -> Result<OAuthTokens, Report<OauthServiceError>> {
        self.oauth_client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.expose_secret().to_owned()))
            .request_async(&ReqwestClient::from(self.http_client.clone()))
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange refresh_token".into(),
            ))
            .map(From::from)
    }

    async fn get_user_info(
        &self,
        access_token: SecretString,
    ) -> Result<OAuthUser, Report<OauthServiceError>> {
        let resp = self
            .http_client
            .get(&self.userinfo_url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", access_token.expose_secret()),
            )
            .send()
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to fetch user info".into(),
            ))?;

        let user = resp
            .json::<crate::model::OidcUserInfo>()
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to deserialize user info response".into(),
            ))?;

        // Treat an explicit `email_verified: false` as a hard failure; absent
        // is tolerated (not all IdPs emit it on the userinfo endpoint).
        if user.email_verified == Some(false) {
            bail!(OauthServiceError::UserEmailNotVerified);
        }

        Ok(user.into())
    }
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
    use egress_http::{EgressClient, EgressClientConfig};
    use secrecy::{ExposeSecret, SecretString};
    use std::str::FromStr;

    fn test_egress() -> EgressClient {
        EgressClient::new(EgressClientConfig::default()).unwrap()
    }

    #[tokio::test]
    async fn test_authorize_url() -> Result<(), Box<dyn std::error::Error>> {
        let srv = OauthServices::new(
            OauthConfig {
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
            },
            test_egress(),
        );

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

    #[tokio::test]
    async fn oidc_sso_authorize_url_has_pkce_and_redirect() {
        let svc = crate::service::build_oidc_sso_service(
            &test_egress(),
            crate::model::OidcSsoConfig {
                client_id: "cid".to_string(),
                client_secret: "secret".to_string(),
                authorize_url: "https://idp.example.com/authorize".to_string(),
                token_url: "https://idp.example.com/token".to_string(),
                userinfo_url: "https://idp.example.com/userinfo".to_string(),
                callback_url: "http://localhost:8080/sso/callback".to_string(),
                scopes: vec!["openid".to_string(), "email".to_string()],
            },
        )
        .unwrap();

        let url = svc.authorize_url();
        let url = url.url.expose_secret();

        assert!(url.starts_with(
            "https://idp.example.com/authorize?response_type=code&client_id=cid&state="
        ));
        assert!(url.contains("code_challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fsso%2Fcallback"));
        assert!(url.contains("scope=openid+email"));
    }

    #[tokio::test]
    async fn well_known_url_building() {
        use crate::service::well_known_url;
        let egress = test_egress();
        let built = |issuer: &str| well_known_url(&egress, issuer).map(|u| u.to_string());
        assert_eq!(
            built("https://accounts.google.com").unwrap(),
            "https://accounts.google.com/.well-known/openid-configuration"
        );
        // Trailing slash trimmed.
        assert_eq!(
            built("https://acme.okta.com/").unwrap(),
            "https://acme.okta.com/.well-known/openid-configuration"
        );
        // Already-complete discovery URL is left as-is.
        assert_eq!(
            built("https://x.com/.well-known/openid-configuration").unwrap(),
            "https://x.com/.well-known/openid-configuration"
        );
        // The egress rules apply to the issuer; destination rules are the edition guard's.
        assert!(built("https://admin:pw@idp.example.com").is_err());
        assert!(built("ftp://idp.example.com").is_err());
    }

    #[tokio::test]
    async fn oidc_sso_rejects_invalid_endpoint_url() {
        let res = crate::service::build_oidc_sso_service(
            &test_egress(),
            crate::model::OidcSsoConfig {
                client_id: "cid".to_string(),
                client_secret: "secret".to_string(),
                authorize_url: "not a url".to_string(),
                token_url: "https://idp.example.com/token".to_string(),
                userinfo_url: "https://idp.example.com/userinfo".to_string(),
                callback_url: "https://app.example.com/sso/callback".to_string(),
                scopes: vec![],
            },
        );
        assert!(res.is_err());
    }
}
