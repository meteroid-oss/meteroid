/// Inspired by https://github.com/mpiorowski/rusve/service-auth
use crate::errors::OauthServiceError;

use crate::model::{CallbackUrl, OAuthUser, OauthProvider};
use async_trait::async_trait;
use error_stack::{ResultExt, bail};
use oauth2::basic::{
    BasicClient, BasicErrorResponse, BasicRevocationErrorResponse, BasicTokenIntrospectionResponse,
    BasicTokenResponse,
};
use oauth2::{
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, PkceCodeChallenge, PkceCodeVerifier, Scope, StandardRevocableToken, TokenResponse,
    TokenUrl,
};
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;

type Oauth2BasicClient = Client<
    BasicErrorResponse,
    BasicTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;

#[async_trait]
pub trait OauthService: Send + Sync {
    fn client_id(&self) -> String;

    fn callback_url(&self) -> CallbackUrl;
    async fn get_user_info(
        &self,
        auth_code: SecretString,
        pkce_code_verifier: SecretString,
    ) -> error_stack::Result<OAuthUser, OauthServiceError>;
}

#[derive(Clone)]
pub struct OauthServices {
    google: Option<Arc<dyn OauthService>>,
}

impl OauthServices {
    pub fn new(config: crate::config::OauthConfig) -> Self {
        Self {
            google: OauthServiceImpl::google(config),
        }
    }

    pub fn for_provider(&self, provider: OauthProvider) -> Option<Arc<dyn OauthService>> {
        match provider {
            OauthProvider::Google => self.google.clone(),
        }
    }

    pub fn client_id(&self, provider: OauthProvider) -> Option<String> {
        match provider {
            OauthProvider::Google => self.google.as_ref().map(|google| google.client_id()),
        }
    }
}

struct OauthServiceImpl {
    config: crate::model::OauthProviderConfig,
    http_client: reqwest::Client,
}

impl OauthServiceImpl {
    fn google(config: crate::config::OauthConfig) -> Option<Arc<dyn OauthService>> {
        if let (Some(client_id), Some(client_secret)) =
            (config.google.client_id, config.google.client_secret)
        {
            Some(Arc::new(Self {
                config: crate::model::OauthProviderConfig {
                    provider: OauthProvider::Google,
                    client_id: client_id.expose_secret().to_owned(),
                    client_secret: client_secret.expose_secret().to_owned(),
                    auth_url: "https://accounts.google.com/o/oauth2/auth".to_owned(),
                    token_url: "https://www.googleapis.com/oauth2/v3/token".to_string(),
                    callback_url: format!("{}/oauth-callback/google", config.public_url.as_str()),
                    user_info_url: "https://www.googleapis.com/oauth2/v3/userinfo".to_string(),
                    scopes: vec!["email".to_string(), "openid".to_string()],
                },
                http_client: reqwest::ClientBuilder::new()
                    // Following redirects opens the client up to SSRF vulnerabilities.
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .expect("Client should build"),
            }))
        } else {
            None
        }
    }

    fn oauth_basic_client(&self) -> Oauth2BasicClient {
        let auth_url = AuthUrl::new(self.config.auth_url.to_owned())
            .expect("Invalid authorization endpoint URL");
        let token_url =
            TokenUrl::new(self.config.token_url.to_owned()).expect("Invalid token endpoint URL");
        let redirect_url = oauth2::RedirectUrl::new(self.config.callback_url.to_owned())
            .expect("Invalid redirect URL");

        BasicClient::new(ClientId::new(self.config.client_id.to_owned()))
            .set_client_secret(ClientSecret::new(self.config.client_secret.to_owned()))
            .set_auth_uri(auth_url)
            .set_token_uri(token_url)
            .set_redirect_uri(redirect_url)
    }

    async fn get_user_info(
        &self,
        token: &str,
    ) -> error_stack::Result<OAuthUser, OauthServiceError> {
        match self.config.provider {
            OauthProvider::Google => {
                let user_profile = self
                    .http_client
                    .get(&self.config.user_info_url)
                    .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
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
        }
    }
}

#[async_trait]
impl OauthService for OauthServiceImpl {
    fn client_id(&self) -> String {
        self.config.client_id.to_owned()
    }

    fn callback_url(&self) -> CallbackUrl {
        // Generate a PKCE challenge.
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        // Generate the full authorization URL.
        let client = self.oauth_basic_client();
        let mut client = client
            .authorize_url(CsrfToken::new_random)
            .set_pkce_challenge(pkce_challenge);

        for scope in &self.config.scopes {
            client = client.add_scope(Scope::new(scope.to_owned()));
        }

        let (auth_url, csrf_token) = client.add_extra_param("access_type", "offline").url();

        CallbackUrl {
            url: SecretString::new(auth_url.to_string()),
            csrf_token: SecretString::new(csrf_token.secret().to_owned()),
            pkce_verifier: SecretString::new(pkce_verifier.secret().to_owned()),
        }
    }

    async fn get_user_info(
        &self,
        auth_code: SecretString,
        pkce_code_verifier: SecretString,
    ) -> error_stack::Result<OAuthUser, OauthServiceError> {
        let client = self.oauth_basic_client();

        let token = client
            .exchange_code(AuthorizationCode::new(auth_code.expose_secret().to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(
                pkce_code_verifier.expose_secret().to_owned(),
            ))
            .request_async(&self.http_client)
            .await
            .change_context(OauthServiceError::ProviderApi(
                "Failed to exchange code".into(),
            ))?;

        self.get_user_info(token.access_token().secret()).await
    }
}

#[cfg(test)]
mod tests {
    use crate::config::OauthConfig;
    use crate::model::OauthProvider;
    use crate::service::OauthServices;
    use secrecy::{ExposeSecret, SecretString};
    use std::str::FromStr;

    #[tokio::test]
    async fn test_signup_callback_url() -> Result<(), Box<dyn std::error::Error>> {
        let srv = OauthServices::new(OauthConfig {
            public_url: "http://localhost:8080".to_string(),
            google: crate::config::GoogleOauthConfig {
                client_id: Some(SecretString::from_str("client_id").unwrap()),
                client_secret: Some(SecretString::from_str("client_secret").unwrap()),
            },
        });

        let url = srv
            .for_provider(OauthProvider::Google)
            .unwrap()
            .callback_url();
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
