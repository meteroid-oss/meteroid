use crate::adapters::stripe::Stripe;
use crate::services::storage::ObjectStoreService;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;

pub mod auth;
mod file_router;
mod webhook_in_router;

pub use file_router::file_routes;
pub use file_router::FileApi;
pub use webhook_in_router::webhook_in_routes;

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStoreService>,
    pub store: Store,
    pub stripe_adapter: Arc<Stripe>,
    pub jwt_secret: SecretString,
}
