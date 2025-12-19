use crate::services::clients::usage::UsageClient;
use crate::{Store, StoreResult};
use chrono::NaiveDateTime;
use common_domain::ids::{InvoiceId, SubscriptionId, TenantId};
use rust_decimal::Decimal;
use std::sync::Arc;

// mod billing_worker;
pub mod utils;

pub mod clients;
mod connectors;
mod credits;
mod edge;
pub mod invoice_lines;
mod invoices;
mod lifecycle;
mod orchestration;
mod payment;
mod quotes;
mod subscriptions;
mod webhooks;

use crate::domain::{PaymentTransaction, Subscription};
pub use crate::domain::{SlotUpgradeBillingMode, UpdateSlotsResult};
pub use invoices::{CustomerDetailsUpdate, InvoiceBillingMode};
pub use quotes::QuoteConversionResult;
use stripe_client::client::StripeClient;
pub use subscriptions::insert::payment_method::PaymentSetupResult;

// INTERNAL. Share connections
#[derive(Clone)]
struct Services {
    store: Arc<Store>,
    usage_client: Arc<dyn UsageClient>,
    pub(crate) stripe: Arc<StripeClient>,
}

// EXTERNAL. Flat api, to be used in apis and workers.
#[derive(Clone)]
pub struct ServicesEdge {
    store: Arc<Store>,
    services: Services,
}

impl ServicesEdge {
    pub fn new(
        store: Arc<Store>,
        usage_client: Arc<dyn UsageClient>,
        stripe: Arc<StripeClient>,
    ) -> Self {
        Self {
            services: Services {
                store: store.clone(),
                usage_client,
                stripe,
            },
            store,
        }
    }

    pub fn usage_clients(&self) -> Arc<dyn UsageClient> {
        self.services.usage_client.clone()
    }

    pub async fn update_subscription_slots(
        &self,
        tenant_id: common_domain::ids::TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
        price_component_id: common_domain::ids::PriceComponentId,
        delta: i32,
        billing_mode: SlotUpgradeBillingMode,
    ) -> StoreResult<UpdateSlotsResult> {
        self.services
            .update_subscription_slots(
                tenant_id,
                subscription_id,
                price_component_id,
                delta,
                billing_mode,
                None,
            )
            .await
    }

    pub async fn activate_pending_slot_transactions(
        &self,
        tenant_id: common_domain::ids::TenantId,
        invoice_id: common_domain::ids::InvoiceId,
        effective_at: Option<NaiveDateTime>,
    ) -> StoreResult<Vec<(common_domain::ids::SubscriptionId, i32)>> {
        self.services
            .activate_pending_slot_transactions(tenant_id, invoice_id, effective_at)
            .await
    }

    pub async fn preview_slot_update(
        &self,
        tenant_id: common_domain::ids::TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
        price_component_id: common_domain::ids::PriceComponentId,
        delta: i32,
    ) -> StoreResult<crate::domain::slot_transactions::SlotUpdatePreview> {
        self.services
            .preview_slot_update(tenant_id, subscription_id, price_component_id, delta)
            .await
    }

    pub async fn complete_slot_upgrade_checkout(
        &self,
        tenant_id: common_domain::ids::TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
        price_component_id: common_domain::ids::PriceComponentId,
        delta: i32,
        payment_method_id: common_domain::ids::CustomerPaymentMethodId,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<(crate::domain::PaymentTransaction, i32)> {
        self.services
            .complete_slot_upgrade_checkout(
                tenant_id,
                subscription_id,
                price_component_id,
                delta,
                payment_method_id,
                at_ts,
            )
            .await
    }

    pub async fn mark_invoice_as_paid(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        total_amount: Decimal,
        payment_date: NaiveDateTime,
        reference: Option<String>,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        self.services
            .mark_invoice_as_paid(tenant_id, invoice_id, total_amount, payment_date, reference)
            .await
    }

    pub async fn add_manual_payment_transaction(
        &self,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        amount: Decimal,
        payment_date: NaiveDateTime,
        reference: Option<String>,
    ) -> StoreResult<PaymentTransaction> {
        self.services
            .add_manual_payment_transaction(tenant_id, invoice_id, amount, payment_date, reference)
            .await
    }

    pub async fn activate_subscription_manual(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription> {
        self.services
            .activate_subscription_manual(tenant_id, subscription_id)
            .await
    }
}

impl ServicesEdge {
    #[cfg(feature = "test-utils")]
    pub async fn update_subscription_slots_for_test(
        &self,
        tenant_id: common_domain::ids::TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
        price_component_id: common_domain::ids::PriceComponentId,
        delta: i32,
        billing_mode: SlotUpgradeBillingMode,
        at_ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<UpdateSlotsResult> {
        self.services
            .update_subscription_slots(
                tenant_id,
                subscription_id,
                price_component_id,
                delta,
                billing_mode,
                at_ts,
            )
            .await
    }
}
