use crate::adapters::stripe::Stripe;
use crate::api_rest::customers::customer_routes;
use crate::api_rest::invoices::invoice_routes;
use crate::api_rest::plans::plan_routes;
use crate::api_rest::productfamilies::product_family_routes;
use crate::api_rest::subscriptions::subscription_routes;
use crate::services::storage::ObjectStoreService;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, de};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use utoipa_axum::router::OpenApiRouter;

mod addresses;
mod auth;
mod currencies;
mod customers;
pub mod error;
mod files;
mod invoices;
mod model;
mod oauth;
pub mod openapi;
mod plans;
mod productfamilies;
pub mod server;
mod subscriptions;
mod webhooks;

pub fn api_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(subscription_routes())
        .merge(product_family_routes())
        .merge(plan_routes())
        .merge(customer_routes())
        .merge(invoice_routes())
}

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStoreService>,
    pub store: Store,
    pub services: Services,
    pub stripe_adapter: Arc<Stripe>,
    pub jwt_secret: SecretString,
}

/// Serde deserialization decorator to map empty Strings to None,
pub fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => FromStr::from_str(s).map_err(de::Error::custom).map(Some),
    }
}
