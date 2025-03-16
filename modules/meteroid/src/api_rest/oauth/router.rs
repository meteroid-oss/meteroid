use crate::api_rest::AppState;
use crate::config::Config;
use axum::extract::{Path, Query, State};
use axum::response::Redirect;
use fang::Deserialize;
use meteroid_oauth::model::OauthProvider;
use meteroid_store::domain::oauth::{OauthVerifierData, SignInData};
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
        .oauth_auth_provider_url(
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
            Redirect::to(error_url(1).as_str())
        }
    }
}

#[axum::debug_handler]
pub async fn callback(
    Path(provider): Path<OauthProvider>,
    Query(params): Query<CallbackParams>,
    State(app_state): State<AppState>,
) -> Redirect {
    let auth_res = app_state
        .store
        .oauth_signin(provider, params.code.into(), params.state.into())
        .await;

    match auth_res {
        Ok(url) => Redirect::to(success_url(&url.token).as_str()),
        Err(e) => {
            log::warn!("Error executing callback: {}", e);
            Redirect::to(error_url(2).as_str())
        }
    }
}

fn error_url(code: u8) -> String {
    format!(
        "{}/error?oauth_signin={}",
        Config::get().public_url.as_str(),
        code
    )
}

fn success_url(token: &SecretString) -> String {
    format!(
        "{}/oauth_success?token{}",
        Config::get().public_url.as_str(),
        token.expose_secret()
    )
}
