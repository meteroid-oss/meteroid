//! Large async fns here wrap their body in `Box::pin(async move { .. }).await`. A caller then
//! embeds a pointer instead of the whole state machine, and its debug-build poll frame stops
//! carrying one stack slot per awaited future; the lifecycle chain otherwise nears 2MB in tests.
use crate::domain::entitlements::EffectiveEntitlement;
use crate::domain::entity_activity::Actor;
use crate::domain::{CheckoutSession, Coupon};
use crate::errors::StoreError;
use crate::repositories::coupons::CouponInterface;
use crate::services::clients::usage::UsageClient;
use crate::{Store, StoreResult};
use chrono::NaiveDateTime;
use common_domain::ids::{CouponId, CustomerId, InvoiceId, SubscriptionId, TenantId};
use diesel_models::applied_coupons::AppliedCouponRow;
use diesel_models::coupons::CouponRow;
use diesel_models::query::coupons::CodeMatch;
use error_stack::Report;
use rust_decimal::Decimal;
use std::sync::Arc;

// mod billing_worker;
pub mod payment_reminders;
pub mod utils;

mod annexes;
mod checkout_completion;
pub use checkout_completion::MAX_DECLINED_CHECKOUT_ATTEMPTS;
mod checkout_preview;
pub mod clients;
pub(crate) mod credit;
mod credit_paid_features;
pub mod customer_emails;
pub mod customer_locale;
mod customer_summary;
mod edge;
pub(crate) mod entitlements;
pub mod fx;
pub(crate) mod intelligence_support;
pub mod invoice_lines;
pub(crate) mod invoices;
mod lifecycle;
pub mod minimum_progress;
pub(crate) mod orchestration;
mod payment;
pub(crate) mod portal_credit;
mod prices;
pub mod promo_code_attempts;
mod quotes;
pub mod sandbox_replay;
pub(crate) mod served_usage;
pub mod subscription_receivables;
mod subscriptions;
mod unattributed_usage;
pub(crate) mod usage_controls_support;
pub mod usage_costs;
pub(crate) mod usage_grouping;
pub mod webhooks;

use crate::domain::{PaymentTransaction, Subscription};
pub use crate::domain::{SlotUpgradeBillingMode, UpdateSlotsResult};
use crate::store::PgConn;
pub use credit::purchase::{SellCustomerCredit, SoldCustomerCredit};
pub use credit::top_up_support::customer_safe_decline_reason;
pub use credit::unit_wallets::{CreditSaleQuote, UnitPriceSource, UnitSale};
pub use credit::wallet_purchase::{ReplayedTopUp, TopUpWalletReceipt};
pub use credit_paid_features::{CreditPaidFeature, CreditPayer};
pub use edge::CheckoutPaymentOutcome;
pub use invoices::{CustomerDetailsUpdate, InvoiceBillingMode, InvoiceDelivery, IssueAt};
pub use lifecycle::CycleTransitionResult;
pub use payment::hosted_payment_sweep::{HostedPaymentSweepOutcome, PendingHostedPaymentRef};
pub use payment::hosted_return::HostedReturnOutcome;
pub use payment::hosted_setup::DefaultMethodAdoption;
pub use payment::hosted_setup::HostedSetupOutcome;
pub use payment::refund::{OfflineRefundCommand, RefundCommand, ReverseManualPaymentCommand};
pub use payment::webhook_backed_setup::WebhookBackedSetupOutcome;
pub use quotes::QuoteConversionResult;
pub use subscriptions::insert::payment_method::PaymentSetupResult;
pub use subscriptions::payment_resolution;
pub use subscriptions::proration;
pub use subscriptions::utils::PendingMaterialization;
pub use subscriptions::utils::validate_charge_automatically_with_provider_ids;

// INTERNAL. Share connections
#[derive(Clone)]
struct Services {
    store: Arc<Store>,
    usage_client: Arc<dyn UsageClient>,
    /// Set while work runs on metering read before its row locks (`served_usage`).
    served: Option<Arc<served_usage::ServedUsage>>,
    /// Per-entity keyset where the last reminder enrolment scan hit its cap (sweep.rs).
    reminder_scan_resume: Arc<
        std::sync::Mutex<
            std::collections::HashMap<
                common_domain::ids::InvoicingEntityId,
                diesel_models::query::payment_reminders::ScanCursor,
            >,
        >,
    >,
}

// EXTERNAL. Flat api, to be used in apis and workers.
#[derive(Clone)]
pub struct ServicesEdge {
    store: Arc<Store>,
    services: Services,
}

impl ServicesEdge {
    pub fn new(store: Arc<Store>, usage_client: Arc<dyn UsageClient>) -> Self {
        Self {
            services: Services {
                store: store.clone(),
                usage_client,
                served: None,
                reminder_scan_resume: Arc::default(),
            },
            store,
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn usage_clients(&self) -> Arc<dyn UsageClient> {
        self.services.usage_client.clone()
    }

    pub async fn get_effective_entitlements(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        self.services
            .get_effective_entitlements(customer_id, tenant_id)
            .await
    }

    pub async fn get_effective_entitlement_for_feature(
        &self,
        customer_id: CustomerId,
        tenant_id: TenantId,
        feature_id: common_domain::ids::FeatureId,
    ) -> StoreResult<Option<EffectiveEntitlement>> {
        self.services
            .get_effective_entitlement_for_feature(customer_id, tenant_id, feature_id)
            .await
    }

    pub async fn get_effective_entitlements_for_subscription(
        &self,
        subscription_id: SubscriptionId,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<EffectiveEntitlement>> {
        self.services
            .get_effective_entitlements_for_subscription(subscription_id, tenant_id)
            .await
    }

    pub async fn list_credit_paid_features(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
    ) -> StoreResult<Vec<CreditPaidFeature>> {
        self.services
            .list_credit_paid_features(tenant_id, customer_id)
            .await
    }

    pub async fn update_subscription_slots(
        &self,
        tenant_id: common_domain::ids::TenantId,
        subscription_id: common_domain::ids::SubscriptionId,
        price_component_id: common_domain::ids::PriceComponentId,
        delta: i32,
        billing_mode: SlotUpgradeBillingMode,
        actor: Actor,
    ) -> StoreResult<UpdateSlotsResult> {
        self.services
            .update_subscription_slots(
                tenant_id,
                subscription_id,
                price_component_id,
                delta,
                billing_mode,
                actor,
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
    ) -> StoreResult<(
        crate::domain::PaymentTransaction,
        i32,
        Option<crate::domain::payment_transactions::PaymentNextAction>,
    )> {
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
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        total_amount: Decimal,
        payment_date: chrono::DateTime<chrono::FixedOffset>,
        reference: Option<String>,
    ) -> StoreResult<crate::domain::DetailedInvoice> {
        self.services
            .mark_invoice_as_paid(
                actor,
                tenant_id,
                invoice_id,
                total_amount,
                payment_date,
                reference,
            )
            .await
    }

    pub async fn add_manual_payment_transaction(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        invoice_id: InvoiceId,
        amount: Decimal,
        payment_date: chrono::DateTime<chrono::FixedOffset>,
        reference: Option<String>,
    ) -> StoreResult<PaymentTransaction> {
        self.services
            .add_manual_payment_transaction(
                actor,
                tenant_id,
                invoice_id,
                amount,
                payment_date,
                reference,
            )
            .await
    }

    /// Refund a settled online payment through its provider. The invoice keeps its balance;
    /// the linked Refund credit note is what reduces it.
    pub async fn refund_transaction(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        command: RefundCommand,
    ) -> StoreResult<PaymentTransaction> {
        self.services
            .refund_transaction(actor, tenant_id, command)
            .await
    }

    /// Record a refund that moved outside Meteroid (wire, cheque, cash).
    pub async fn record_offline_refund(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        command: OfflineRefundCommand,
    ) -> StoreResult<PaymentTransaction> {
        self.services
            .record_offline_refund(actor, tenant_id, command)
            .await
    }

    /// Claw back a manually recorded payment in full; the invoice reopens.
    pub async fn reverse_manual_payment(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        command: ReverseManualPaymentCommand,
    ) -> StoreResult<PaymentTransaction> {
        self.services
            .reverse_manual_payment(actor, tenant_id, command)
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

    /// Set or replace a subscription-level minimum commitment as an operator override
    /// (`is_override = true`, kept across plan changes), restating the MRR it floors.
    pub async fn set_subscription_minimum(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        amount_subunit: i64,
        scope: crate::domain::plan_minimums::MinimumScope,
    ) -> StoreResult<crate::domain::SubscriptionMinimum> {
        self.services
            .set_subscription_minimum(tenant_id, subscription_id, amount_subunit, scope)
            .await
    }

    /// Remove the subscription-level minimum (no floor until the next plan change re-snapshots
    /// the plan's), restating the MRR.
    pub async fn clear_subscription_minimum(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        self.services
            .clear_subscription_minimum(tenant_id, subscription_id)
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
    #[doc(hidden)]
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
                Actor::System,
                at_ts,
            )
            .await
    }
}

impl Services {
    /// The coupons a checkout is about to charge with: the session's `coupon_ids`, else the coupon
    /// its code names (`coupon_code_override`, else the session's), resolved as `CodeMatch` does.
    /// A code naming no live coupon, or several, is refused before any charge.
    pub(crate) async fn resolve_coupon_ids_for_checkout_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        session: &CheckoutSession,
        coupon_code_override: Option<String>,
    ) -> StoreResult<Vec<CouponId>> {
        if !session.coupon_ids.is_empty() {
            return Ok(session.coupon_ids.clone());
        }
        match checkout_coupon_code(session, coupon_code_override) {
            Some(code) => Ok(vec![
                self.checkout_coupon_by_code_tx(conn, tenant_id, &code)
                    .await?
                    .id,
            ]),
            None => Ok(vec![]),
        }
    }

    /// The live coupon a code typed or set on a checkout names, as `CodeMatch` resolves it.
    pub(crate) async fn checkout_coupon_by_code_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<Coupon> {
        let rows = CouponRow::candidates_by_codes(conn, tenant_id, &[code.to_owned()]).await?;
        let found = match CodeMatch::of(code, &rows) {
            CodeMatch::Found(row) => row.id,
            CodeMatch::Unknown => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Coupon code '{}' is not valid",
                    code.trim()
                ))));
            }
            CodeMatch::Ambiguous => {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Coupon code '{}' matches several coupons, enter it exactly as you were given it",
                    code.trim()
                ))));
            }
        };
        rows.into_iter()
            .find(|row| row.id == found)
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(format!("coupon {}", code.trim())))
            })?
            .try_into()
    }

    /// The coupons of a checkout whose payment already moved. Its coupons were persisted on the
    /// session when it was charged; a session charged by an older version without them keeps
    /// its earlier outcome (an unresolved code applies no coupon) rather than failing a payment.
    pub(crate) async fn paid_checkout_coupon_ids_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        session: &CheckoutSession,
    ) -> StoreResult<Vec<CouponId>> {
        if !session.coupon_ids.is_empty() {
            return Ok(session.coupon_ids.clone());
        }
        let Some(code) = checkout_coupon_code(session, None) else {
            return Ok(vec![]);
        };
        Ok(self
            .store
            .list_coupons_by_codes_tx(conn, tenant_id, &[code])
            .await?
            .into_iter()
            .map(|coupon| coupon.id)
            .collect())
    }

    /// Locks coupons with FOR UPDATE and validates they can be used.
    /// Validates:
    /// - Coupon is not expired, archived, or disabled
    /// - Coupon has not reached its redemption limit
    /// - Coupon currency matches subscription currency (if applicable)
    /// - Non-reusable and promotional credit coupons haven't been used by this customer before
    /// - Plan restrictions, and not already applied to the subscription
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn lock_and_validate_coupons_for_checkout(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        customer_id: CustomerId,
        coupon_ids: &[CouponId],
        subscription_currency: &str,
        plan_version_id: Option<common_domain::ids::PlanVersionId>,
        subscription_id: Option<SubscriptionId>,
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

        let restricted =
            diesel_models::coupon_plans::CouponPlanRow::list_by_coupon_ids(conn, coupon_ids)
                .await?;
        if restricted.values().any(|plans| !plans.is_empty()) {
            let plan_id = match plan_version_id {
                Some(version_id) => {
                    diesel_models::plan_versions::PlanVersionRow::get_plan_ids_by_version_ids(
                        conn,
                        &[version_id],
                    )
                    .await?
                    .remove(&version_id)
                }
                None => None,
            };
            for coupon in &coupons {
                if let Some(plans) = restricted.get(&coupon.id)
                    && !plans.is_empty()
                    && !plan_id.is_some_and(|plan_id| plans.contains(&plan_id))
                {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Coupon {} cannot be applied to this plan",
                        coupon.code
                    ))));
                }
            }
        }

        if let Some(subscription_id) = subscription_id {
            let applied =
                diesel_models::applied_coupons::AppliedCouponDetailedRow::list_by_subscription_id(
                    conn,
                    &subscription_id,
                )
                .await?;
            if let Some(coupon) = coupons.iter().find(|coupon| {
                applied
                    .iter()
                    .any(|row| row.applied_coupon.coupon_id == coupon.id)
            }) {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Coupon {} is already applied to this subscription",
                    coupon.code
                ))));
            }
        }

        // Promotional credit is redeemed once per customer, whatever `reusable` says.
        let once_per_customer: Vec<CouponId> = coupons
            .iter()
            .filter(|c| !c.reusable || c.discount.is_credit())
            .map(|c| c.id)
            .collect();

        if !once_per_customer.is_empty() {
            let pairs: Vec<(CouponId, CustomerId)> = once_per_customer
                .iter()
                .map(|&id| (id, customer_id))
                .collect();

            let existing =
                AppliedCouponRow::find_existing_customer_coupon_pairs(conn, &pairs).await?;

            for coupon in coupons.iter().filter(|c| once_per_customer.contains(&c.id)) {
                if existing.contains(&(coupon.id, customer_id)) {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "Coupon {} has already been used by this customer",
                        coupon.code
                    ))));
                }
            }
        }

        // A checkout whose payment still settles applies its coupons only then: it holds the
        // redemption already. The coupon rows locked above serialize concurrent checkouts.
        use diesel_models::checkout_sessions::CheckoutSessionRow;
        for coupon in &coupons {
            if once_per_customer.contains(&coupon.id)
                && CheckoutSessionRow::count_awaiting_payment_with_coupon(
                    conn,
                    tenant_id,
                    coupon.id,
                    Some(customer_id),
                )
                .await?
                    > 0
            {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Coupon {} has already been used by this customer",
                    coupon.code
                ))));
            }
            if let Some(limit) = coupon.redemption_limit
                && i64::from(coupon.redemption_count)
                    + CheckoutSessionRow::count_awaiting_payment_with_coupon(
                        conn, tenant_id, coupon.id, None,
                    )
                    .await?
                    >= i64::from(limit)
            {
                return Err(Report::new(StoreError::InvalidArgument(format!(
                    "Coupon '{}' has reached its redemption limit",
                    coupon.code
                ))));
            }
        }

        Ok(coupons)
    }
}

pub(crate) fn checkout_coupon_code(
    session: &CheckoutSession,
    coupon_code_override: Option<String>,
) -> Option<String> {
    coupon_code_override
        .or_else(|| session.coupon_code.clone())
        .filter(|code| !code.trim().is_empty())
}
