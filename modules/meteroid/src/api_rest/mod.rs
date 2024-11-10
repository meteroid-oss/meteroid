use crate::adapters::stripe::Stripe;
use crate::services::storage::ObjectStoreService;
use axum::Router;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;

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
