use crate::adapters::stripe::Stripe;
use crate::api_rest::customers::customer_routes;
use crate::api_rest::events::event_routes;
use crate::api_rest::invoices::invoice_routes;
use crate::api_rest::plans::plan_routes;
use crate::api_rest::productfamilies::product_family_routes;
use crate::api_rest::subscriptions::subscription_routes;
use crate::services::storage::ObjectStoreService;
use axum::extract::FromRequestParts;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use http::request::Parts;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use serde::de::DeserializeOwned;
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
mod events;
mod files;
mod invoices;
mod metrics;
mod model;
mod oauth;
pub mod openapi;
mod plans;
mod productfamilies;
pub mod server;
mod subscriptions;
pub mod webhooks;

pub fn api_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(subscription_routes())
        .merge(product_family_routes())
        .merge(plan_routes())
        .merge(customer_routes())
        .merge(invoice_routes())
        .merge(event_routes())
}

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStoreService>,
    pub store: Store,
    pub services: Services,
    pub stripe_adapter: Arc<Stripe>,
    pub jwt_secret: SecretString,
    pub portal_url: String,
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

/// Custom Query extractor that uses serde_qs for proper array parameter handling.
/// This supports repeated query params like `status=A&status=B` being parsed as Vec.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryParams<T>(pub T);

#[derive(Debug)]
pub struct QueryParamsRejection(String);

impl IntoResponse for QueryParamsRejection {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.0).into_response()
    }
}

impl<T, S> FromRequestParts<S> for QueryParams<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = QueryParamsRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        serde_html_form::from_str(query)
            .map(QueryParams)
            .map_err(|e| QueryParamsRejection(format!("Failed to deserialize query string: {}", e)))
    }
}

impl<T> axum_valid::HasValidate for QueryParams<T> {
    type Validate = T;
    fn get_validate(&self) -> &T {
        &self.0
    }
}
