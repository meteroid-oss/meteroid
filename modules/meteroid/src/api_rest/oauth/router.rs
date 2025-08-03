use crate::api_rest::AppState;
use crate::config::Config;
use crate::errors::RestApiError;
use axum::extract::{Path, Query, State};
use axum::response::Redirect;
use fang::Deserialize;
use meteroid_oauth::model::OauthProvider;
use meteroid_store::domain::oauth::{OauthVerifierData, SignInData};
use meteroid_store::repositories::TenantInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::oauth::OauthInterface;
use meteroid_store::repositories::users::UserInterface;
use secrecy::{ExposeSecret, SecretString};

#[derive(Deserialize)]
pub struct GetCallbackUrlParams {
    is_signup: bool,
    invite_key: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String,
}

#[axum::debug_handler]
pub async fn redirect_to_identity_provider(
    Path(provider): Path<OauthProvider>,
    Query(params): Query<GetCallbackUrlParams>,
    State(app_state): State<AppState>,
) -> Redirect {
    let callback_url_res = app_state
        .store
        .oauth_auth_url(
            provider,
            OauthVerifierData::SignIn(SignInData {
                is_signup: params.is_signup,
                invite_key: params.invite_key,
            }),
        )
        .await;

    match callback_url_res {
        Ok(url) => Redirect::to(url.expose_secret()),
        Err(e) => {
            log::warn!("Error getting callback URL: {}", e);
            Redirect::to(signin_error_url(1).as_str())
        }
    }
}

#[axum::debug_handler]
pub async fn callback(
    Path(provider): Path<OauthProvider>,
    Query(params): Query<CallbackParams>,
    State(app_state): State<AppState>,
) -> Result<Redirect, RestApiError> {
    match provider {
        OauthProvider::Google => Ok(signin_callback(provider, params, app_state).await),
        OauthProvider::Hubspot => {
            oauth_connect_callback(OauthProvider::Hubspot, params, app_state).await
        }
        OauthProvider::Pennylane => {
            oauth_connect_callback(OauthProvider::Pennylane, params, app_state).await
        }
    }
}

async fn oauth_connect_callback(
    oauth_provider: OauthProvider,
    params: CallbackParams,
    app_state: AppState,
) -> Result<Redirect, RestApiError> {
    let connected = app_state
        .store
        .connect_oauth(oauth_provider, params.code.into(), params.state.into())
        .await;

    match connected {
        Ok(conn) => {
            let tenant = app_state
                .store
                .find_tenant_by_id(conn.connector.tenant_id)
                .await
                .map_err(RestApiError::from)?;

            let section = match oauth_provider {
                OauthProvider::Hubspot => "#crm",
                OauthProvider::Pennylane => "#accounting",
                _ => "",
            };

            let url = format!(
                "{}/{}/{}/settings?success=true&tab=integrations{}",
                Config::get().public_url,
                tenant.organization.slug,
                tenant.tenant.slug,
                section
            );

            Ok(Redirect::to(url.as_str()))
        }
        Err(e) => {
            log::warn!("Error connecting {}: {}", oauth_provider, e);
            Err(RestApiError::from(e))
        }
    }
}

async fn signin_callback(
    provider: OauthProvider,
    params: CallbackParams,
    app_state: AppState,
) -> Redirect {
    let auth_res = app_state
        .store
        .oauth_signin(provider, params.code.into(), params.state.into())
        .await;

    match auth_res {
        Ok(url) => Redirect::to(signin_success_url(&url.token).as_str()),
        Err(e) => {
            log::warn!("Error executing callback: {}", e);
            Redirect::to(signin_error_url(2).as_str())
        }
    }
}

fn signin_error_url(code: u8) -> String {
    format!(
        "{}/error?oauth_signin={}",
        Config::get().public_url.as_str(),
        code
    )
}

fn signin_success_url(token: &SecretString) -> String {
    format!(
        "{}/oauth_success?token={}",
        Config::get().public_url.as_str(),
        token.expose_secret()
    )
}
