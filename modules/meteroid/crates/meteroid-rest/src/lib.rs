use crate::addons::addon_routes;
use crate::batch_jobs::batch_job_routes;
use crate::checkoutsessions::checkout_session_routes;
use crate::coupons::coupon_routes;
use crate::credit_units::credit_unit_routes;
use crate::creditnotes::credit_note_routes;
use crate::credits::credit_routes;
use crate::customers::customer_routes;
use crate::customproperties::custom_property_routes;
use crate::entitlements::{
    effective_entitlement_routes, entitlement_routes, entity_entitlement_routes, feature_routes,
};
use crate::events::event_routes;
use crate::invoices::invoice_routes;
use crate::metrics::metric_routes;
use crate::plans::plan_routes;
use crate::productfamilies::product_family_routes;
use crate::products::product_routes;
use crate::rate_limit::RateLimiters;
use crate::subscriptions::subscription_routes;
use crate::usage::usage_routes;
use crate::usage_costs::usage_cost_routes;
use crate::webhooks::endpoints::webhook_endpoint_routes;
use meteroid_app::services::storage::ObjectStoreService;
use meteroid_store::{Services, Store};
use secrecy::SecretString;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use utoipa_axum::router::OpenApiRouter;

pub mod webhook_catalog;

pub mod addons;
pub mod addresses;
pub mod auth;
pub mod batch_jobs;
pub mod checkoutsessions;
pub mod client;
pub mod coupons;
pub mod credit_units;
pub mod creditnotes;
pub mod credits;
pub mod currencies;
pub mod customers;
pub mod customproperties;
pub mod entitlements;
pub mod events;
pub mod files;
pub mod hosted_return;
pub mod invoices;
pub mod metrics;
pub mod mock;
pub mod oauth;
pub mod openapi;
pub mod plans;
pub mod portal_links;
pub mod productfamilies;
pub mod products;
pub mod rate_limit;
pub mod sso;
pub mod subscriptions;
pub mod usage;
pub mod usage_costs;
pub mod webhooks;

pub fn api_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(addon_routes())
        .merge(batch_job_routes())
        .merge(subscription_routes())
        .merge(product_family_routes())
        .merge(plan_routes())
        .merge(customer_routes())
        .merge(custom_property_routes())
        .merge(invoice_routes())
        .merge(credit_note_routes())
        .merge(credit_unit_routes())
        .merge(checkout_session_routes())
        .merge(event_routes())
        .merge(usage_routes())
        .merge(credit_routes())
        .merge(usage_cost_routes())
        .merge(product_routes())
        .merge(coupon_routes())
        .merge(metric_routes())
        .merge(feature_routes())
        .merge(entitlement_routes())
        .merge(entity_entitlement_routes())
        .merge(effective_entitlement_routes())
        .merge(webhook_endpoint_routes())
}

#[derive(Clone)]
pub struct AppState {
    pub object_store: Arc<dyn ObjectStoreService>,
    pub store: Store,
    pub services: Services,
    pub jwt_secret: SecretString,
    pub portal_url: String,
    pub api_url: String,
    /// `REST_API_EXTERNAL_URL` as configured, for the redirect URIs identity providers register.
    pub rest_api_external_url: String,
    pub trusted_proxy_hops: usize,
    pub ready: Arc<AtomicBool>,
    pub rest_rate_limiters: RateLimiters,
    pub feature_resolver: Arc<dyn meteroid_app::features::Entitlements>,
    pub subscription_usage: usage::model::SubscriptionUsageCache,
    /// Shared SSRF guard for webhook destinations, mirroring the delivery worker's client.
    pub webhook_egress: egress_http::EgressClient,
    pub webhook_out: meteroid_app::config::WebhookOutConfig,
}
