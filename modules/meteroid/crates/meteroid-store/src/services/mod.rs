use crate::domain::{CheckoutSession, Coupon};
use crate::errors::StoreError;
use crate::repositories::coupons::CouponInterface;
use crate::services::clients::usage::UsageClient;
use crate::{Store, StoreResult};
use chrono::NaiveDateTime;
use common_domain::ids::{CouponId, CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::applied_coupons::AppliedCouponRow;
use diesel_models::coupons::CouponRow;
use error_stack::Report;
use rust_decimal::Decimal;
use std::sync::Arc;

// mod billing_worker;
pub mod utils;

mod checkout_completion;
mod checkout_preview;
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
mod prices;
mod webhooks;

use crate::domain::{PaymentTransaction, Subscription};
pub use crate::domain::{SlotUpgradeBillingMode, UpdateSlotsResult};
use crate::store::PgConn;
pub use invoices::{CustomerDetailsUpdate, InvoiceBillingMode};
pub use lifecycle::CycleTransitionResult;
pub use quotes::QuoteConversionResult;
use stripe_client::client::StripeClient;
pub use subscriptions::insert::payment_method::PaymentSetupResult;
pub use subscriptions::payment_resolution;
pub use subscriptions::utils::validate_charge_automatically_with_provider_ids;

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

    /// Resolves the effective plan for a subscription based on its trial status.
    ///
    /// Returns information about which plan should be "in effect" for billing/features:
    /// - During active trial with trialing_plan_id: returns the trialing plan
    /// - After trial expired with DOWNGRADE action: returns the downgrade plan
    /// - Otherwise: returns the subscription's original plan
    pub async fn get_subscription_effective_plan(
        &self,
        conn: &mut crate::store::PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<crate::domain::EffectivePlanInfo> {
        self.services
            .get_subscription_effective_plan(conn, tenant_id, subscription_id)
            .await
    }

    /// Builds a virtual SubscriptionDetails from a checkout session for invoice preview.
    ///
    /// This is used in the self-serve checkout flow where there's no subscription yet.
    /// The returned SubscriptionDetails can be used with compute_invoice() to preview
    /// the invoice that would be generated when the checkout is completed.
    pub async fn build_preview_subscription_details(
        &self,
        session: &crate::domain::CheckoutSession,
        tenant_id: TenantId,
        coupon_code: Option<&str>,
    ) -> StoreResult<crate::domain::SubscriptionDetails> {
        let mut conn = self.store.get_conn().await?;
        self.services
            .build_preview_subscription_details(&mut conn, session, tenant_id, coupon_code)
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

impl Services {
    /// Resolves coupon IDs for a checkout session.
    ///
    /// If the session already has `coupon_ids`, returns those.
    /// Otherwise, if a `coupon_code_override` is provided or the session has a `coupon_code`,
    /// looks up the coupon by code and returns its ID.
    pub(crate) async fn resolve_coupon_ids_for_checkout_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        session: &CheckoutSession,
        coupon_code_override: Option<String>,
    ) -> StoreResult<Vec<CouponId>> {
        let mut coupon_ids = session.coupon_ids.clone();

        if coupon_ids.is_empty() {
            // Use override if provided, otherwise fall back to session's coupon_code
            let effective_code = coupon_code_override.or_else(|| session.coupon_code.clone());

            if let Some(code) = effective_code {
                let coupons = self
                    .store
                    .list_coupons_by_codes_tx(conn, tenant_id, &[code])
                    .await?;
                if let Some(coupon) = coupons.into_iter().next() {
                    coupon_ids.push(coupon.id);
                }
            }
        }

        Ok(coupon_ids)
    }

    /// Locks coupons with FOR UPDATE and validates they can be used.
    /// Validates:
    /// - Coupon is not expired, archived, or disabled
    /// - Coupon has not reached its redemption limit
    /// - Coupon currency matches subscription currency (if applicable)
    /// - Non-reusable coupons haven't been used by this customer before
    pub(crate) async fn lock_and_validate_coupons_for_checkout(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        coupon_ids: &[CouponId],
        subscription_currency: &str,
    ) -> StoreResult<Vec<Coupon>> {
        if coupon_ids.is_empty() {
            return Ok(vec![]);
        }

        // Lock coupons with FOR UPDATE to prevent concurrent modifications
        let coupon_rows = CouponRow::list_by_ids_for_update(conn, coupon_ids, &tenant_id).await?;

        // Convert to domain objects and validate
        let mut coupons = Vec::with_capacity(coupon_rows.len());
        for row in coupon_rows {
            let coupon: Coupon = row.try_into()?;

            // Validate coupon can be used (expired, archived, disabled, redemption limit, currency)
            coupon
                .validate_for_use_with_message(subscription_currency)
                .map_err(|msg| Report::new(StoreError::InvalidArgument(msg)))?;

            coupons.push(coupon);
        }

        // Check non-reusable coupons haven't been used by this customer
        let non_reusable_ids: Vec<CouponId> = coupons
            .iter()
            .filter(|c| !c.reusable)
            .map(|c| c.id)
            .collect();

        if !non_reusable_ids.is_empty() {
            let pairs: Vec<(CouponId, CustomerId)> = non_reusable_ids
                .iter()
                .map(|&id| (id, customer_id))
                .collect();

            let existing =
                AppliedCouponRow::find_existing_customer_coupon_pairs(conn, &pairs).await?;

            for coupon in coupons.iter().filter(|c| !c.reusable) {
                if existing.contains(&(coupon.id, customer_id)) {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Coupon {} is not reusable and has already been used by this customer",
                        coupon.code
                    ))));
                }
            }
        }

        Ok(coupons)
    }
}
