pub mod add_ons;
pub mod api_tokens;
pub mod applied_coupons;
pub mod bi;
pub mod billable_metrics;
pub mod configs;
pub mod coupons;
pub mod customer_balance_txs;
pub mod customers;
pub mod historical_rates_from_usd;
pub mod invoices;
pub mod invoicing_entities;
pub mod organization_members;
pub mod organizations;
pub mod outbox_event;
pub mod plan_versions;
pub mod plans;
pub mod price_components;
pub mod product_families;
pub mod products;
pub mod schedules;
pub mod slot_transactions;
pub mod stats;
pub mod subscription_add_ons;
pub mod subscription_components;
pub mod subscription_events;
pub mod subscriptions;
pub mod tenants;
pub mod users;
pub mod webhooks;

pub enum IdentityDb {
    UUID(uuid::Uuid),
    LOCAL(String),
}
