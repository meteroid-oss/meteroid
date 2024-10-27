use crate::adapters::stripe::Stripe;
use crate::services::storage::ObjectStoreService;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use common_grpc::middleware::server::AuthorizedState;
use http::StatusCode;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;
use uuid::Uuid;

pub mod auth;
mod file_router;
mod model;
mod subscription_router;
mod webhook_in_router;

pub use file_router::file_routes;
pub use file_router::FileApi;
pub use webhook_in_router::webhook_in_routes;

pub fn api_routes() -> Router<AppState> {
    Router::new().merge(get(subscription_router::subscription_routes()))
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

pub fn extract_maybe_tenant(
    maybe_auth: Option<&AuthorizedState>,
) -> Result<Uuid, Response<String>> {
    let authorized = maybe_auth.ok_or(
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body("Missing authorized state in request extensions".into()),
    )?;

    let res = match authorized {
        AuthorizedState::Tenant { tenant_id, .. } => { Ok(*tenant_id) }
        AuthorizedState::Organization { .. } => {
            Err(
                Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body("Tenant is absent from the authorized state. This indicates an incomplete x-md-context header.".into())
            )
        }
        AuthorizedState::User { .. } => {
            Err(
                Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body("Tenant is absent from the authorized state. This indicates a missing x-md-context header.".into())
            )
        }
    }?;

    Ok(res)
}
