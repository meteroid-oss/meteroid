//! Store internals the usage-controls module's live engine is built on: invoice-time pricing and
//! limit rules, the ledger and its allocation, forecasts, and the outbox and customer emails it
//! writes to. Internal to the usage-controls module; nothing else may depend on this module.

pub use crate::services::credit::{invalid, precondition};
pub use crate::services::usage_controls_support::{
    EngineServices, charge_queued_automatic_top_ups, once,
};

pub mod allocation {
    pub use crate::repositories::credit::allocation::Allocation;
    pub use crate::repositories::credit::allocation::CreditDemand;
    pub use crate::repositories::credit::allocation::CreditRequest;
    pub use crate::repositories::credit::allocation::DemandScope;
    pub use crate::repositories::credit::allocation::GrantCandidate;
    pub use crate::repositories::credit::allocation::GrantPool;
    pub use crate::repositories::credit::allocation::allocate;
    pub use crate::repositories::credit::allocation::allocate_in_turn_keeping;
    pub use crate::repositories::credit::allocation::assign;
    pub use crate::repositories::credit::allocation::rollover_stand_in;
}

pub mod billing_windows {
    pub use crate::services::credit::billing_windows::Charge;
    pub use crate::services::credit::billing_windows::MoneyOverage;
    pub use crate::services::credit::billing_windows::PlannedWindow;
    pub use crate::services::credit::billing_windows::charged_denominations;
    pub use crate::services::credit::billing_windows::price_window_groups;
    pub use crate::services::credit::billing_windows::subscription_windows;
    pub use crate::services::credit::billing_windows::subscriptions_windows;
}

pub mod customer_emails {
    pub use crate::repositories::customer_emails::enqueue_customer_email_on_tx;
}

pub mod denomination {
    pub use crate::repositories::credit::denomination::account_unit_precision;
}

pub mod forecast {
    pub use crate::services::credit::forecast::CustomerCreditForecast;
    pub use crate::services::credit::forecast::ForecastDocument;
    pub use crate::services::credit::forecast::ForecastUntil;
    pub use crate::services::credit::forecast::MAX_FORECAST_SUBSCRIPTIONS;
    pub use crate::services::credit::forecast::once_grants_tx;
    pub use crate::services::credit::forecast::plan_names_tx;
    pub use crate::services::credit::forecast::row_group;
}

pub mod invoice_lines {
    pub use crate::services::invoice_lines::component::PinnedPrices;
    pub use crate::services::invoice_lines::credit_units::allocate_covered;
    pub use crate::services::invoice_lines::invoice_lines::MeteredFee;
}

pub mod ledger {
    pub use crate::repositories::credit::customer_credit::DEFAULT_PRIORITY;
    pub use crate::repositories::credit::customer_credit::PendingDemand;
    pub use crate::repositories::credit::customer_credit::UsageCharge;
    pub use crate::repositories::credit::customer_credit::UsageOwed;
    pub use crate::repositories::credit::customer_credit::after_paid_settlements;
    pub use crate::repositories::credit::customer_credit::available_minor_tx;
    pub use crate::repositories::credit::customer_credit::candidate;
    pub use crate::repositories::credit::customer_credit::ended_first;
    pub use crate::repositories::credit::customer_credit::grant_products_tx;
    pub use crate::repositories::credit::customer_credit::lock_customer_tx;
    pub use crate::repositories::credit::customer_credit::normalized_currency;
}

pub mod limit_rules {
    pub use crate::services::entitlements::limits::rules::BillingCyclePeriod;
    pub use crate::services::entitlements::limits::rules::compute_usage_period;
    pub use crate::services::entitlements::limits::rules::crossed_pcts;
    pub use crate::services::entitlements::limits::rules::group_key;
    pub use crate::services::entitlements::limits::rules::hard_ceiling;
    pub use crate::services::entitlements::limits::rules::held_pcts;
    pub use crate::services::entitlements::limits::rules::ungroupable_reason;
    pub use crate::services::entitlements::limits::rules::window_key;
}

pub mod limits {
    pub use crate::services::entitlements::limits::anchor_subscription;
    pub use crate::services::entitlements::limits::customer_entitlement_monitor_id;
    pub use crate::services::entitlements::limits::earliest_activation;
    pub use crate::services::entitlements::limits::entitlement_monitor_id;
    pub use crate::services::entitlements::limits::group_usage;
    pub use crate::services::entitlements::limits::load_metrics;
}

pub mod outbox {
    pub use crate::repositories::outbox::publish_outbox_events_tx;
}

pub mod portal_credit {
    pub use crate::services::portal_credit::MIN_PROJECTION_ELAPSED_SECONDS;
}

pub mod scope {
    pub use crate::services::credit::scope::LineOwners;
    pub use crate::services::credit::scope::line_subscription;
}

pub mod top_up_support {
    pub use crate::services::credit::top_up_support::GENERIC_DECLINE;
    pub use crate::services::credit::top_up_support::customer_safe_decline_reason;
    pub use crate::services::credit::top_up_support::metadata_object;
    pub use crate::services::credit::top_up_support::minimum_charge_minor;
    pub use crate::services::credit::top_up_support::truncate_error;
}

pub mod unit_wallets {
    pub use crate::services::credit::unit_wallets::UNIT_SALE_KEY;
    pub use crate::services::credit::unit_wallets::UnitSale;
    pub use crate::services::credit::unit_wallets::charge_estimate_minor;
    pub use crate::services::credit::unit_wallets::pinned_unit_sale;
    pub use crate::services::credit::unit_wallets::resolve_unit_sale;
}

pub mod upcoming {
    pub use crate::services::invoices::upcoming::scheduled_stop;
}

pub mod usage_attribution {
    pub use crate::repositories::subscriptions::usage_attribution::attributed_subscriptions;
    pub use crate::repositories::subscriptions::usage_attribution::scopes_of_row;
}

pub mod usage_grouping {
    pub use crate::services::usage_grouping::MAX_GROUP_KEY_BYTES;
    pub use crate::services::usage_grouping::dimensions_of;
    pub use crate::services::usage_grouping::foldable;
    pub use crate::services::usage_grouping::group_key;
    pub use crate::services::usage_grouping::metric_dimensions;
}

pub mod value_application {
    pub use crate::services::credit::value_application::quoted_subtotal;
    pub use crate::services::credit::value_application::usage_gross_subtotal;
}

pub mod wallet_purchase {
    pub use crate::services::credit::wallet_purchase::AUTOMATIC_TOP_UP_NAMESPACE;
}
