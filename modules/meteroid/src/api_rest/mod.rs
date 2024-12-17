use crate::adapters::stripe::Stripe;
use crate::api_rest::plans::plan_routes;
use crate::api_rest::productfamilies::product_families_routes;
use crate::api_rest::subscriptions::subscription_routes;
use crate::services::storage::ObjectStoreService;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;
use utoipa_axum::router::OpenApiRouter;

mod auth;
mod files;
mod model;
pub mod openapi;
mod plans;
mod productfamilies;
pub mod server;
mod subscriptions;
mod webhooks;

pub fn api_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(subscription_routes())
        .merge(product_families_routes())
        .merge(plan_routes())
}

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStoreService>,
    pub store: Store,
    pub stripe_adapter: Arc<Stripe>,
    pub jwt_secret: SecretString,
}
