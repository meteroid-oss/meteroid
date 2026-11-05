#![recursion_limit = "256"]

/// Logs a query's SQL and binds at debug level, only with the `debug-sql` feature: binds include
/// secrets (tokens, password hashes). Without it, it still proves `query` a Pg `QueryFragment`
/// first, which halves the type checking of a query met for the first time.
#[inline(always)]
pub fn debug_sql<Q: diesel::query_builder::QueryFragment<diesel::pg::Pg>>(query: &Q) {
    #[cfg(feature = "debug-sql")]
    log::debug!("{}", diesel::debug_query::<diesel::pg::Pg, _>(query));
    #[cfg(not(feature = "debug-sql"))]
    let _ = query;
}

pub mod api_idempotency;
pub mod api_tokens;
pub mod batch_jobs;
pub mod bi;
pub mod billable_metrics;
pub mod connectors;
pub mod credit_inputs;
pub mod credit_notes;
pub mod credit_units;
pub mod custom_property_definitions;
pub mod customer_connection;
pub mod customers;
pub mod entitlements;
pub use diesel_schema::enums;
pub mod errors;
pub mod instance_telemetry;
pub mod invoices;
pub mod lock;
pub mod org_security_audit_log;
pub mod organization_invites;
pub mod organization_members;
pub mod organizations;
pub mod plan_component_prices;
pub mod plan_version_adjustments;
pub mod plan_versions;
pub mod plans;
pub mod portal_tokens;
pub mod price_components;
pub mod prices;
pub mod product_families;
pub mod production_revenue;
pub mod products;
pub mod promo_code_attempts;
pub mod query;
pub mod quote_add_ons;
pub mod quote_adjustments;
pub mod quote_coupons;
pub mod quotes;
pub mod replay_holds;
pub mod revenue_stats;
pub mod roles;
pub mod schedules;
pub use diesel_schema::schema;
pub mod slot_transactions;
pub mod sso;
pub mod subscriptions;

pub mod accounting;
pub mod add_ons;
pub mod applied_coupons;
pub mod bank_accounts;
pub mod checkout_sessions;
pub mod coupon_plans;
pub mod coupons;
pub mod customer_credit;
pub mod customer_payment_methods;
pub mod dead_letter;
pub mod email_kind_audience;
pub mod email_send_claim;
pub mod entity_activity;
pub mod entity_list_views;
pub mod extend;
pub mod historical_rates_from_usd;
pub mod invoice_annex_templates;
pub mod invoicing_entities;
pub mod invoicing_entity_email_settings;
pub mod oauth_verifiers;
pub mod outbox_event;
pub mod payment_reminders;
pub mod payment_reversal_movements;
pub mod payments;
pub mod pgmq;
pub mod plan_version_add_ons;
pub mod purchase_links;
pub mod sandbox_seed;
pub mod scheduled_events;
pub mod sent_email;
pub mod stats;
pub mod stored_object;
pub mod subscription_add_ons;
pub mod subscription_adjustments;
pub mod subscription_components;
pub mod subscription_events;
pub mod subscription_usage_filters;
pub mod tax_categories;
pub mod tenants;
pub mod users;
pub mod webhooks;
pub mod webhooks_out;

use diesel_async::pooled_connection::deadpool::Object;

use crate::errors::DatabaseErrorContainer;
use diesel_async::AsyncPgConnection;

pub type DbResult<T> = Result<T, DatabaseErrorContainer>;

pub type PgConn = Object<AsyncPgConnection>;

pub mod aliases {}
