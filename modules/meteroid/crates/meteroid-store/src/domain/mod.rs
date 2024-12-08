pub use api_tokens::*;
pub use billable_metrics::*;
pub use customers::*;
use diesel_models::query::IdentityDb;
pub use invoice_lines::*;
pub use invoices::*;
pub use invoicing_entities::*;
pub use misc::*;
use o2o::o2o;
pub use organizations::*;
pub use plans::*;
pub use price_components::*;
pub use product_families::*;
pub use products::*;
pub use schedules::*;
pub use subscription_add_ons::*;
pub use subscription_components::*;
pub use subscription_coupons::*;
pub use subscriptions::*;
pub use tenants::*;
use uuid::Uuid;

pub mod customers;
pub mod invoices;
pub mod plans;

pub mod price_components;
pub mod tenants;

pub mod add_ons;
pub mod adjustments;
pub mod api_tokens;
pub mod billable_metrics;
pub mod configs;
pub mod coupons;
pub mod enums;
pub mod historical_rates;
pub mod invoice_lines;
pub mod invoicing_entities;
pub mod misc;
pub mod organizations;
pub mod outbox_event;
pub mod product_families;
pub mod products;
pub mod schedules;
pub mod stats;
pub mod subscription_add_ons;
pub mod subscription_components;
pub mod subscription_coupons;
pub mod subscriptions;
pub mod users;
pub mod webhooks;

#[derive(Debug, Clone, PartialEq, Eq, o2o)]
#[owned_into(IdentityDb)]
pub enum Identity {
    UUID(Uuid),
    LOCAL(String),
}
