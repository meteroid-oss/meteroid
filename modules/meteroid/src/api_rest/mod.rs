use crate::adapters::stripe::Stripe;
use crate::services::storage::ObjectStoreService;
use axum::response::{IntoResponse, Response};
use axum::Router;
use common_grpc::middleware::server::AuthorizedState;
use http::StatusCode;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;
use uuid::Uuid;

mod auth;
mod files;
mod model;
pub mod server;
mod subscriptions;
mod webhooks;

pub fn api_routes() -> Router<AppState> {
    Router::new().merge(subscriptions::subscription_routes())
}

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStoreService>,
    pub store: Store,
    pub stripe_adapter: Arc<Stripe>,
    pub jwt_secret: SecretString,
}

pub fn extract_tenant(auth: AuthorizedState) -> Result<Uuid, Response> {
    extract_maybe_tenant(Some(&auth))
}

pub fn extract_maybe_tenant(maybe_auth: Option<&AuthorizedState>) -> Result<Uuid, Response> {
    let authorized = maybe_auth.ok_or(
        (
            StatusCode::UNAUTHORIZED,
            "Missing authorized state in request extensions",
        )
            .into_response(),
    )?;

    let res = match authorized {
        AuthorizedState::Tenant { tenant_id, .. } => { Ok(*tenant_id) }
        AuthorizedState::Organization { .. } => {
            Err(
                (StatusCode::UNAUTHORIZED, "Tenant is absent from the authorized state. This indicates an incomplete x-md-context header.").into_response()
            )
        }
        AuthorizedState::User { .. } => {
            Err(
                (StatusCode::UNAUTHORIZED, "Tenant is absent from the authorized state. This indicates a missing x-md-context header.").into_response()
            )
        }
    }?;

    Ok(res)
}
