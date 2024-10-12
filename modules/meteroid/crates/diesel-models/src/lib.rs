pub mod api_tokens;
pub mod bi;
pub mod billable_metrics;
pub mod configs;
pub mod credit_notes;
pub mod customers;
pub mod enums;
pub mod errors;
pub mod fang;
pub mod invoices;
pub mod organization_members;
pub mod organizations;
pub mod plan_versions;
pub mod plans;
pub mod price_components;
pub mod product_families;
pub mod products;
pub mod query;
pub mod schedules;
pub mod schema;
pub mod slot_transactions;
pub mod subscriptions;

pub mod add_ons;
pub mod coupons;
pub mod customer_balance_txs;
pub mod extend;
pub mod historical_rates_from_usd;
pub mod invoicing_entities;
pub mod outbox;
pub mod stats;
pub mod subscription_add_ons;
pub mod subscription_components;
pub mod subscription_coupons;
pub mod subscription_events;
pub mod tenants;
pub mod users;
pub mod webhooks;

use diesel_async::pooled_connection::deadpool::Object;

use crate::errors::DatabaseErrorContainer;
use diesel_async::AsyncPgConnection;

pub type DbResult<T> = Result<T, DatabaseErrorContainer>;

pub type PgConn = Object<AsyncPgConnection>;
