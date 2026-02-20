use super::context::SubscriptionCreationContext;
use super::payment_method::PaymentSetupResult;
use crate::constants::{Currencies, Currency};
use crate::domain::checkout_sessions::{CheckoutType, CreateCheckoutSession};
use crate::domain::coupons::Coupon;
use crate::domain::enums::SubscriptionEventType;
use crate::domain::scheduled_events::ScheduledEventNew;
use crate::domain::slot_transactions::{SlotTransaction, SlotTransactionNewInternal};
use crate::domain::{
    CreateSubscription, CreateSubscriptionAddOns, CreateSubscriptionComponents,
    CreateSubscriptionFromQuote, CreatedSubscription, Customer, PaymentMethodsConfig,
    SlotTransactionStatusEnum, SubscriptionActivationCondition, SubscriptionAddOnNew,
    SubscriptionAddOnNewInternal, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionNew, SubscriptionNewEnriched, SubscriptionStatusEnum,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::jwt_claims::{ResourceAccess, generate_portal_token};
use crate::services::InvoiceBillingMode;
use crate::services::subscriptions::utils::{
    PendingMaterialization, apply_coupons, apply_coupons_without_validation, calculate_mrr,
    extract_billing_period, process_create_subscription_add_ons,
    process_create_subscription_components, process_create_subscription_coupons,
};
use crate::store::PgConn;
use crate::utils::periods::{
    calculate_advance_period_range, calculate_elapsed_cycles, find_period_containing_date,
};
use crate::{StoreResult, services::Services};
use chrono::{Datelike, NaiveDate};
use common_domain::ids::{BaseId, QuoteId, SubscriptionId, TenantId};
use common_eventbus::{Event, EventBus};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::checkout_sessions::{CheckoutSessionRow, CheckoutSessionRowNew};
use diesel_models::enums::CycleActionEnum;
use diesel_models::plans::PlanRow;
use diesel_models::scheduled_events::ScheduledEventRowNew;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use error_stack::{Report, ResultExt};
use futures::TryFutureExt;
use secrecy::SecretString;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::log;
use uuid::Uuid;
// PROCESS

#[derive(Debug)]
pub struct ProcessedSubscription {
    subscription: SubscriptionRowNew,
    components: Vec<SubscriptionComponentRowNew>,
    add_ons: Vec<SubscriptionAddOnRowNew>,
    coupons: Vec<AppliedCouponRowNew>,
    event: SubscriptionEventRow,
    slot_transactions: Vec<SlotTransactionRow>,
    scheduled_events: Vec<ScheduledEventNew>,
    /// Products/prices that need to be created inside the persist transaction.
    pending_materializations: Vec<PendingMaterialization>,
    /// When true, skip checkout session creation even if pending_checkout is true.
    /// Used for subscriptions created from checkout completion (SelfServe flow).
    skip_checkout_session: bool,
    /// When true, skip billing for this subscription (migration mode).
    skip_past_invoices: bool,
}

pub struct DetailedSubscription {
    pub subscription: SubscriptionNew,
    components: Vec<SubscriptionComponentNewInternal>,
    add_ons: Vec<SubscriptionAddOnNewInternal>,
    coupons: Vec<Coupon>,
    pub customer: Customer,
    currency: Currency,
    pub slot_transactions: Vec<SlotTransactionNewInternal>,
    pub pending_materializations: Vec<PendingMaterialization>,
}

impl Services {
    pub(crate) fn build_subscription_details(
        &self,
        batch: &[CreateSubscription],
        context: &SubscriptionCreationContext,
    ) -> StoreResult<Vec<DetailedSubscription>> {
        let mut results = Vec::new();

        for (idx, params) in batch.iter().enumerate() {
            let CreateSubscription {
                subscription,
                price_components,
                add_ons,
                coupons,
            } = params;

            let customer = context
                .customers
                .iter()
                .find(|c| c.id == subscription.customer_id)
                .ok_or(Report::new(StoreError::InsertError))
                .attach("Customer not found")?;

            let plan = context
                .plans
                .iter()
                .find(|p| p.version_id == subscription.plan_version_id)
                .ok_or(Report::new(StoreError::ValueNotFound(
                    "Plan id not found".to_string(),
                )))?;

            let subscription_currency = &plan.currency.clone();

            let currency = Currencies::resolve_currency(subscription_currency)
                .ok_or(StoreError::InsertError)
                .attach("Failed to resolve currency")?
                .clone();

            let resolved = context
                .resolved_custom_components
                .get(idx)
                .ok_or(Report::new(StoreError::InsertError))
                .attach("Missing resolved custom components for batch index")?;

            let (components, pending_materializations) =
                self.process_components(price_components, subscription, context, resolved, plan)?;
            let subscription_add_ons = self.process_add_ons(add_ons, context)?;

            let slot_transactions = process_slot_transactions(
                &components,
                &subscription_add_ons,
                subscription.start_date,
            )?;

            let coupons_resolved = if let Some(coupons) = coupons {
                context
                    .all_coupons
                    .iter()
                    .filter(|c| {
                        coupons
                            .coupons
                            .iter()
                            .any(|coupon| c.id == coupon.coupon_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };

            results.push(DetailedSubscription {
                subscription: subscription.clone(),
                components,
                add_ons: subscription_add_ons,
                coupons: coupons_resolved,
                customer: customer.clone(),
                currency,
                slot_transactions,
                pending_materializations,
            });
        }

        Ok(results)
    }

    pub(crate) fn build_subscription_details_from_quote(
        &self,
        params: &CreateSubscriptionFromQuote,
        context: &SubscriptionCreationContext,
    ) -> StoreResult<DetailedSubscription> {
        let subscription = &params.subscription;

        let customer = context
            .customers
            .iter()
            .find(|c| c.id == subscription.customer_id)
            .ok_or(Report::new(StoreError::InsertError))
            .attach("Customer not found")?;

        let plan = context
            .plans
            .iter()
            .find(|p| p.version_id == subscription.plan_version_id)
            .ok_or(Report::new(StoreError::ValueNotFound(
                "Plan id not found".to_string(),
            )))?;

        let subscription_currency = &plan.currency.clone();

        let currency = Currencies::resolve_currency(subscription_currency)
            .ok_or(StoreError::InsertError)
            .attach("Failed to resolve currency")?
            .clone();

        let components = params.components.clone();
        let add_ons = params.add_ons.clone();

        let slot_transactions =
            process_slot_transactions(&components, &add_ons, subscription.start_date)?;

        let coupons: Vec<Coupon> = context
            .all_coupons
            .iter()
            .filter(|c| params.coupon_ids.contains(&c.id))
            .cloned()
            .collect();

        Ok(DetailedSubscription {
            subscription: subscription.clone(),
            components,
            add_ons,
            coupons,
            customer: customer.clone(),
            currency,
            slot_transactions,
            pending_materializations: vec![], // Quotes have pre-resolved products/prices
        })
    }

    pub(crate) fn process_subscription(
        &self,
        sub: &DetailedSubscription,
        payment_setup_result: &PaymentSetupResult,
        context: &SubscriptionCreationContext,
        tenant_id: TenantId,
        quote_id: Option<QuoteId>,
    ) -> StoreResult<ProcessedSubscription> {
        let period = extract_billing_period(&sub.components, &sub.add_ons);

        let plan = context
            .plans
            .iter()
            .find(|p| p.version_id == sub.subscription.plan_version_id)
            .ok_or(Report::new(StoreError::ValueNotFound(
                "Plan id not found".to_string(),
            )))?;

        let subscription_id = SubscriptionId::new();

        let billing_start_date = sub
            .subscription
            .billing_start_date
            .unwrap_or(sub.subscription.start_date);

        let now = chrono::Utc::now().naive_utc();

        // Validate: skip_past_invoices only makes sense with a past start_date
        if sub.subscription.skip_past_invoices && sub.subscription.start_date >= now.date() {
            return Err(Report::new(StoreError::InvalidArgument(
                "skip_past_invoices requires a past start_date".to_string(),
            )));
        }

        // Validate: skip_past_invoices only works with OnStart activation
        if sub.subscription.skip_past_invoices
            && sub.subscription.activation_condition != SubscriptionActivationCondition::OnStart
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "skip_past_invoices requires activation_condition to be OnStart".to_string(),
            )));
        }

        // Use trial_duration from request, or fall back to plan's trial_duration_days
        // Filter out 0 values - a trial of 0 days means no trial
        let effective_trial_duration: Option<u32> = sub
            .subscription
            .trial_duration
            .or(plan.trial_duration_days.map(|d| d as u32))
            .filter(|&d| d > 0);

        // Billing day anchor priority:
        // 1. Explicit anchor (fixed day billing)
        // 2. Trial end date day (anniversary billing with FREE trial - billing starts after trial)
        // 3. Billing start date day (paid trial or no trial - billing starts immediately)
        let billing_day_anchor = sub.subscription.billing_day_anchor.unwrap_or_else(|| {
            // For FREE trials, anchor to trial end date (billing starts after trial ends)
            // For PAID trials, anchor to billing start date (billing starts immediately)
            let has_free_trial_for_anchor =
                effective_trial_duration.is_some() && plan.trial_is_free;
            if has_free_trial_for_anchor {
                let trial_days = effective_trial_duration.unwrap();
                let trial_end = billing_start_date + chrono::Duration::days(i64::from(trial_days));
                trial_end.day() as u16
            } else {
                billing_start_date.day() as u16
            }
        });

        let net_terms = sub.subscription.net_terms.unwrap_or(plan.net_terms as u32);

        let activated_at = match sub.subscription.activation_condition {
            SubscriptionActivationCondition::OnStart => {
                sub.subscription.start_date.and_hms_opt(0, 0, 0)
            }
            _ => None,
        };

        let mut current_period_start = billing_start_date;
        let mut current_period_end = None;
        let mut next_cycle_action = None;
        let mut cycle_index = None;
        let mut status = SubscriptionStatusEnum::PendingActivation;
        let mut scheduled_events: Vec<ScheduledEventNew> = vec![];
        let mut imported_at = None;

        // Handle trial: distinguish between free and paid trials
        // - Free trial: billing period = trial duration, no billing until trial ends
        // - Paid trial: billing period = full cycle (e.g., 30 days), billing immediately
        //   Trial only affects feature resolution (trialing_plan_id), not billing
        let has_free_trial = effective_trial_duration.is_some() && plan.trial_is_free;
        let has_paid_trial = effective_trial_duration.is_some() && !plan.trial_is_free;

        if sub.subscription.activation_condition == SubscriptionActivationCondition::OnStart {
            if sub.subscription.start_date <= now.date() {
                // Migration mode: skip past billing, set up subscription at current point
                if sub.subscription.skip_past_invoices {
                    imported_at = Some(now);

                    // Calculate effective billing start (post-trial for free trials)
                    let effective_billing_start = if has_free_trial {
                        let trial_days = effective_trial_duration.unwrap();
                        billing_start_date + chrono::Duration::days(i64::from(trial_days))
                    } else {
                        billing_start_date
                    };

                    // Check if trial is still ongoing
                    let trial_end_date = if has_free_trial || has_paid_trial {
                        Some(
                            billing_start_date
                                + chrono::Duration::days(i64::from(
                                    effective_trial_duration.unwrap(),
                                )),
                        )
                    } else {
                        None
                    };
                    let trial_still_active =
                        trial_end_date.map(|d| d > now.date()).unwrap_or(false);

                    if trial_still_active {
                        // Trial is still ongoing - set up as trial subscription
                        let trial_days = effective_trial_duration.unwrap();
                        status = SubscriptionStatusEnum::TrialActive;
                        cycle_index = Some(0);
                        current_period_start = billing_start_date;
                        current_period_end = Some(
                            billing_start_date + chrono::Duration::days(i64::from(trial_days)),
                        );
                        next_cycle_action = Some(CycleActionEnum::EndTrial);
                        // For paid trial, also schedule the EndTrial event
                        if has_paid_trial {
                            let scheduled_event = ScheduledEventNew::end_trial(
                                subscription_id,
                                tenant_id,
                                billing_start_date,
                                trial_days as i32,
                                "subscription_creation_migration",
                            )
                            .ok_or_else(|| {
                                Report::new(StoreError::InvalidArgument(
                                    "Failed to compute trial end date".to_string(),
                                ))
                            })?;
                            scheduled_events.push(scheduled_event);
                        }
                    } else {
                        // Trial has ended (or no trial) - calculate current period based on today
                        let elapsed = calculate_elapsed_cycles(
                            effective_billing_start,
                            now.date(),
                            &period,
                            u32::from(billing_day_anchor),
                        );
                        let current = find_period_containing_date(
                            effective_billing_start,
                            now.date(),
                            &period,
                            u32::from(billing_day_anchor),
                        );

                        status = SubscriptionStatusEnum::Active;

                        // If today is exactly a renewal boundary (period starts today and at least
                        // one cycle has elapsed), use the ending period instead of the new one.
                        // This lets the automation pick it up and create the invoice for the
                        // completed period.
                        if current.start == now.date() && elapsed > 0 {
                            let prev = find_period_containing_date(
                                effective_billing_start,
                                now.date() - chrono::Duration::days(1),
                                &period,
                                u32::from(billing_day_anchor),
                            );
                            cycle_index = Some(elapsed - 1);
                            current_period_start = prev.start;
                            current_period_end = Some(prev.end);
                        } else {
                            cycle_index = Some(elapsed);
                            current_period_start = current.start;
                            current_period_end = Some(current.end);
                        }
                        next_cycle_action = Some(CycleActionEnum::RenewSubscription);
                    }
                } else if has_free_trial {
                    // Free trial: period = trial duration, no billing until trial ends
                    let trial_days = effective_trial_duration.unwrap();
                    status = SubscriptionStatusEnum::TrialActive;
                    cycle_index = Some(0); // Track cycle from start, even during trial
                    current_period_start = billing_start_date;
                    current_period_end =
                        Some(current_period_start + chrono::Duration::days(i64::from(trial_days)));
                    next_cycle_action = Some(CycleActionEnum::EndTrial);
                } else if has_paid_trial {
                    // Paid trial: period = full billing cycle, bill immediately
                    // Trial only affects feature resolution, not billing
                    // Use normal RenewSubscription cycle - trial end is handled via scheduled event
                    let trial_days = effective_trial_duration.unwrap();
                    let range = calculate_advance_period_range(
                        billing_start_date,
                        u32::from(billing_day_anchor),
                        true,
                        &period,
                    );
                    status = SubscriptionStatusEnum::TrialActive;
                    cycle_index = Some(0);
                    current_period_start = range.start;
                    current_period_end = Some(range.end);
                    // Use RenewSubscription for normal billing cycle
                    next_cycle_action = Some(CycleActionEnum::RenewSubscription);

                    // Schedule trial end event at start_date + trial_duration
                    let scheduled_event = ScheduledEventNew::end_trial(
                        subscription_id,
                        tenant_id,
                        billing_start_date,
                        trial_days as i32,
                        "subscription_creation",
                    )
                    .ok_or_else(|| {
                        Report::new(StoreError::InvalidArgument(
                            "Failed to compute trial end date".to_string(),
                        ))
                    })?;
                    scheduled_events.push(scheduled_event);
                } else {
                    // No trial: normal billing
                    let range = calculate_advance_period_range(
                        billing_start_date,
                        u32::from(billing_day_anchor),
                        true,
                        &period,
                    );

                    status = SubscriptionStatusEnum::Active;
                    cycle_index = Some(0);
                    current_period_start = range.start;
                    current_period_end = Some(range.end);
                    next_cycle_action = Some(CycleActionEnum::RenewSubscription);
                }
            } else {
                current_period_end = Some(sub.subscription.start_date);
                next_cycle_action = Some(CycleActionEnum::ActivateSubscription);
            }
        } else if sub.subscription.activation_condition
            == SubscriptionActivationCondition::OnCheckout
            && has_free_trial
        {
            // OnCheckout with free trial: start the trial immediately
            // Checkout will be required when trial ends
            if sub.subscription.start_date <= now.date() {
                let trial_days = effective_trial_duration.unwrap(); // Safe: has_free_trial implies this
                status = SubscriptionStatusEnum::TrialActive;
                cycle_index = Some(0); // Track cycle from start, even during trial
                current_period_start = billing_start_date;
                current_period_end =
                    Some(current_period_start + chrono::Duration::days(i64::from(trial_days)));
                next_cycle_action = Some(CycleActionEnum::EndTrial);
            } else {
                current_period_end = Some(sub.subscription.start_date);
                next_cycle_action = Some(CycleActionEnum::ActivateSubscription);
            }
        }
        // OnCheckout with paid trial: stays PendingActivation, checkout will handle billing

        let enriched = SubscriptionNewEnriched {
            subscription: &sub.subscription,
            subscription_id,
            tenant_id,
            period,
            plan,
            payment_setup_result,
            billing_day_anchor,
            billing_start_date,
            status,
            current_period_start,
            current_period_end,
            next_cycle_action,
            activated_at,
            net_terms,
            cycle_index,
            quote_id,
            effective_trial_duration,
            imported_at,
        };

        let subscription_row = enriched.map_to_row()?;

        let subscription_coupons = self.process_coupons(&subscription_row, &sub.coupons)?;

        let event = self.build_subscription_event(
            &subscription_row,
            &sub.components,
            &sub.add_ons,
            &context.all_coupons,
            sub.currency.precision,
        )?;

        let components = sub
            .components
            .iter()
            .map(|c| {
                SubscriptionComponentNew {
                    subscription_id: subscription_row.id,
                    internal: c.clone(),
                }
                .try_into()
            })
            .collect::<std::result::Result<Vec<_>, StoreErrorReport>>()?;

        let subscription_add_ons = sub
            .add_ons
            .iter()
            .map(|internal| {
                SubscriptionAddOnNew {
                    subscription_id: subscription_row.id,
                    internal: internal.clone(),
                }
                .try_into()
            })
            .collect::<Result<Vec<_>, StoreErrorReport>>()?;

        let slot_transactions = sub
            .slot_transactions
            .iter()
            .map(|tx| {
                SlotTransaction {
                    id: tx.id,
                    subscription_id: subscription_row.id,
                    unit: tx.unit.clone(),
                    delta: tx.delta,
                    prev_active_slots: tx.prev_active_slots,
                    effective_at: tx.effective_at,
                    transaction_at: tx.transaction_at,
                    status: SlotTransactionStatusEnum::Active,
                    invoice_id: None,
                }
                .into()
            })
            .collect::<Vec<_>>();

        Ok(ProcessedSubscription {
            subscription: subscription_row,
            components,
            add_ons: subscription_add_ons,
            coupons: subscription_coupons,
            event,
            slot_transactions,
            scheduled_events,
            pending_materializations: sub.pending_materializations.clone(),
            skip_checkout_session: sub.subscription.skip_checkout_session,
            skip_past_invoices: sub.subscription.skip_past_invoices,
        })
    }

    fn process_components(
        &self,
        components: &Option<CreateSubscriptionComponents>,
        subscription: &SubscriptionNew,
        context: &SubscriptionCreationContext,
        resolved: &super::context::ResolvedCustomComponents,
        plan: &crate::domain::PlanForSubscription,
    ) -> Result<
        (
            Vec<SubscriptionComponentNewInternal>,
            Vec<PendingMaterialization>,
        ),
        StoreErrorReport,
    > {
        process_create_subscription_components(
            components,
            &context.price_components_by_plan_version,
            subscription,
            &context.products_by_id,
            resolved,
            plan.product_family_id,
            &plan.currency,
        )
    }

    fn process_add_ons(
        &self,
        add_ons: &Option<CreateSubscriptionAddOns>,
        context: &SubscriptionCreationContext,
    ) -> Result<Vec<SubscriptionAddOnNewInternal>, StoreErrorReport> {
        process_create_subscription_add_ons(
            add_ons,
            &context.all_add_ons,
            &context.products_by_id,
            &context.addon_prices_by_id,
        )
    }

    fn process_coupons(
        &self,
        subscription: &SubscriptionRowNew,
        coupons: &[Coupon],
    ) -> Result<Vec<AppliedCouponRowNew>, StoreErrorReport> {
        process_create_subscription_coupons(subscription, coupons)
    }

    fn build_subscription_event(
        &self,
        subscription: &SubscriptionRowNew,
        components: &[SubscriptionComponentNewInternal],
        add_ons: &[SubscriptionAddOnNewInternal],
        _coupons: &[Coupon],
        precision: u8,
    ) -> Result<SubscriptionEventRow, StoreErrorReport> {
        let cmrr: i64 = components
            .iter()
            .map(|c| calculate_mrr(&c.fee, &c.period, precision))
            .sum();

        let ao_mrr: i64 = add_ons
            .iter()
            .map(|c| calculate_mrr(&c.fee, &c.period, precision) * c.quantity as i64)
            .sum();

        let mrr_delta = cmrr + ao_mrr;

        // TODO w need a single request for all possible currency conversions here (or just filter out), and reuse the conn
        let final_mrr = mrr_delta;
        // let final_mrr = calculate_coupons_discount(
        //     self,
        //     coupons,
        //     &subscription.currency,
        //     Decimal::from_i64(mrr_delta).unwrap_or(Decimal::ZERO),
        // )
        // .await?
        // .to_i64()
        // .unwrap_or(0);

        Ok(SubscriptionEventRow {
            id: Uuid::now_v7(),
            subscription_id: subscription.id,
            event_type: SubscriptionEventType::Created.into(),
            details: None,
            created_at: chrono::Utc::now().naive_utc(),
            mrr_delta: Some(final_mrr),
            bi_mrr_movement_log_id: None,
            applies_to: subscription.start_date,
        })
    }
}

fn process_slot_transactions(
    components: &[SubscriptionComponentNewInternal],
    addons: &[SubscriptionAddOnNewInternal],
    start_date: NaiveDate,
) -> StoreResult<Vec<SlotTransactionNewInternal>> {
    let mut transactions = vec![];

    for component in components {
        if let Some(tx) = SlotTransactionNewInternal::from_fee(&component.fee, start_date) {
            transactions.push(tx);
        }
    }

    for addon in addons {
        if let Some(tx) = SlotTransactionNewInternal::from_fee(&addon.fee, start_date) {
            transactions.push(tx);
        }
    }

    Ok(transactions)
}

// PERSIST

impl Services {
    pub(crate) async fn persist_subscriptions(
        &self,
        conn: &mut PgConn,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        self.persist_subscriptions_internal(
            conn, processed, tenant_id, jwt_secret, public_url, false,
        )
        .await
    }

    /// Persist subscriptions without coupon validation.
    ///
    /// Use this for async payment flows where coupons were already validated before charging.
    pub(crate) async fn persist_subscriptions_skip_coupon_validation(
        &self,
        conn: &mut PgConn,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        self.persist_subscriptions_internal(
            conn, processed, tenant_id, jwt_secret, public_url, true,
        )
        .await
    }

    async fn persist_subscriptions_internal(
        &self,
        conn: &mut PgConn,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
        skip_coupon_validation: bool,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let res = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    // Materialize pending products/prices inside the transaction.
                    // Builds a map of (sub_idx, comp_idx) → (product_id, price_id) for patching.
                    let mut materialized: HashMap<
                        (usize, usize),
                        (
                            common_domain::ids::ProductId,
                            Option<common_domain::ids::PriceId>,
                        ),
                    > = HashMap::new();
                    for (sub_idx, proc) in processed.iter().enumerate() {
                        for mat in &proc.pending_materializations {
                            use crate::domain::price_components::PriceComponentNewInternal;
                            use crate::repositories::price_components::resolve_component_internal;

                            let internal = PriceComponentNewInternal {
                                name: mat.name.clone(),
                                product_ref: mat.product_ref.clone(),
                                prices: vec![mat.price_entry.clone()],
                            };
                            let (product_id, price_ids) = resolve_component_internal(
                                conn,
                                &internal,
                                tenant_id,
                                proc.subscription.created_by,
                                mat.product_family_id,
                                &mat.currency,
                            )
                            .await?;

                            materialized.insert(
                                (sub_idx, mat.component_index),
                                (product_id, price_ids.into_iter().next()),
                            );
                        }
                    }

                    // Flatten collections for batch insertion, patching materialized components.
                    let subscriptions: Vec<_> = processed.iter().map(|p| &p.subscription).collect();

                    let mut patched_components: Vec<SubscriptionComponentRowNew> = Vec::new();
                    for (sub_idx, proc) in processed.iter().enumerate() {
                        for (comp_idx, comp) in proc.components.iter().enumerate() {
                            if let Some((product_id, price_id)) =
                                materialized.get(&(sub_idx, comp_idx))
                            {
                                let mut patched = comp.clone();
                                patched.product_id = Some(*product_id);
                                patched.price_id = *price_id;
                                patched_components.push(patched);
                            } else {
                                patched_components.push(comp.clone());
                            }
                        }
                    }
                    let components: Vec<_> = patched_components.iter().collect();
                    let add_ons: Vec<_> = processed.iter().flat_map(|p| &p.add_ons).collect();
                    let coupons: Vec<_> = processed.iter().flat_map(|p| &p.coupons).collect();
                    let events: Vec<_> = processed.iter().map(|p| &p.event).collect();
                    let slot_transactions: Vec<_> = processed
                        .iter()
                        .flat_map(|p| &p.slot_transactions)
                        .collect();

                    // Perform batch insertions
                    let inserted: Vec<CreatedSubscription> =
                        SubscriptionRow::insert_subscription_batch(conn, subscriptions)
                            .await
                            .map(|v| v.into_iter().map(Into::into).collect())?;

                    SubscriptionComponentRow::insert_subscription_component_batch(conn, components)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    SubscriptionAddOnRow::insert_batch(conn, add_ons)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    if skip_coupon_validation {
                        apply_coupons_without_validation(conn, &coupons)
                            .map_err(Into::<StoreErrorReport>::into)
                            .await?;
                    } else {
                        apply_coupons(conn, &coupons, &inserted, tenant_id)
                            .map_err(Into::<StoreErrorReport>::into)
                            .await?;
                    }

                    SubscriptionEventRow::insert_batch(conn, events)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    SlotTransactionRow::insert_batch(conn, slot_transactions)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    // Insert scheduled events (e.g., EndTrial for paid trials)
                    let scheduled_events: Vec<ScheduledEventRowNew> = processed
                        .iter()
                        .flat_map(|p| &p.scheduled_events)
                        .cloned()
                        .map(|e| e.try_into())
                        .collect::<Result<Vec<_>, _>>()?;
                    if !scheduled_events.is_empty() {
                        ScheduledEventRowNew::insert_batch(conn, &scheduled_events)
                            .map_err(Into::<StoreErrorReport>::into)
                            .await?;
                    }

                    self.insert_created_outbox_events_tx(conn, &inserted, tenant_id)
                        .await?;

                    // For pending_checkout subscriptions, create checkout sessions inside the transaction
                    // so the FK constraint on subscription_id is satisfied.
                    // Skip if skip_checkout_session is true (subscription created from checkout completion).
                    for (sub, proc) in inserted.iter().zip(processed.iter()) {
                        if sub.pending_checkout && !proc.skip_checkout_session {
                            // Deserialize payment_methods_config from the processed subscription
                            let payment_methods_config: Option<PaymentMethodsConfig> = proc
                                .subscription
                                .payment_methods_config
                                .as_ref()
                                .map(|v| serde_json::from_value(v.clone()))
                                .transpose()
                                .map_err(|e| {
                                    StoreError::SerdeError("payment_methods_config".to_string(), e)
                                })?;

                            let create_session = CreateCheckoutSession {
                                tenant_id,
                                customer_id: sub.customer_id,
                                plan_version_id: sub.plan_version_id,
                                created_by: sub.created_by,
                                billing_start_date: sub.billing_start_date,
                                billing_day_anchor: Some(sub.billing_day_anchor),
                                net_terms: Some(sub.net_terms),
                                trial_duration_days: sub.trial_duration,
                                end_date: sub.end_date,
                                auto_advance_invoices: true,
                                charge_automatically: true,
                                invoice_memo: sub.invoice_memo.clone(),
                                invoice_threshold: sub.invoice_threshold,
                                purchase_order: sub.purchase_order.clone(),
                                payment_methods_config,
                                components: None,
                                add_ons: None,
                                coupon_code: None,
                                coupon_ids: vec![],
                                expires_in_hours: None,
                                metadata: None,
                                checkout_type: CheckoutType::SubscriptionActivation,
                                subscription_id: Some(sub.id),
                            };

                            let session_row: CheckoutSessionRowNew =
                                create_session.try_into_row()?;
                            session_row
                                .insert(conn)
                                .await
                                .map_err(|e| StoreError::DatabaseError(e.error))?;
                        }
                    }

                    for (sub, proc) in inserted.iter().zip(processed.iter()) {
                        // Skip billing if subscription is not activated or pending checkout
                        if sub.activated_at.is_none() || sub.pending_checkout {
                            continue;
                        }

                        // Skip billing for migration mode subscriptions.
                        // The subscription is set up with current_period_end = today,
                        // so the cycle processing worker will pick it up and generate
                        // the invoice for the next period.
                        if proc.skip_past_invoices {
                            continue;
                        }

                        // For subscriptions with trials, check if it's a free trial
                        if sub.trial_duration.is_some() {
                            // Fetch plan version to check trial_is_free
                            let plan_with_version =
                                PlanRow::get_with_version(conn, sub.plan_version_id, tenant_id)
                                    .await?;

                            // Skip billing for free trials
                            if let Some(version) = plan_with_version.version
                                && version.trial_is_free
                            {
                                continue;
                            }
                            // Paid trial: fall through to billing
                        }

                        self.bill_subscription_tx(
                            conn,
                            tenant_id,
                            sub.id,
                            InvoiceBillingMode::Immediate,
                        )
                        .await?;
                    }

                    Ok::<_, StoreErrorReport>(inserted)
                }
                .scope_boxed()
            })
            .await?;

        // For pending_checkout subscriptions, generate checkout URLs
        // (sessions were already created inside the transaction above)
        let mut inserted_with_checkout_urls = Vec::with_capacity(res.len());

        for mut sub in res {
            if sub.pending_checkout {
                // Query for the checkout session that was created for this subscription
                let mut conn = self.store.get_conn().await?;
                if let Some(session) =
                    CheckoutSessionRow::get_by_subscription(&mut conn, tenant_id, sub.id)
                        .await
                        .ok()
                        .flatten()
                {
                    let token = generate_portal_token(
                        jwt_secret,
                        tenant_id,
                        ResourceAccess::CheckoutSession(session.id),
                    )?;
                    let checkout_url = format!("{}/checkout?token={}", public_url, token);
                    sub.checkout_url = Some(checkout_url);
                }
            }
            inserted_with_checkout_urls.push(sub);
        }

        Ok(inserted_with_checkout_urls)
    }

    pub async fn handle_post_insertion(
        &self,
        event_bus: Arc<dyn EventBus<Event>>,
        inserted: &[CreatedSubscription],
    ) -> StoreResult<()> {
        // Publish events
        self.publish_subscription_events(event_bus, inserted)
            .await?;

        Ok(())
    }

    async fn publish_subscription_events(
        &self,
        event_bus: Arc<dyn EventBus<Event>>,
        subscriptions: &[CreatedSubscription],
    ) -> StoreResult<()> {
        let results = futures::future::join_all(subscriptions.iter().map(|sub| {
            event_bus.publish(Event::subscription_created(
                sub.created_by,
                sub.id.as_uuid(),
                sub.tenant_id.as_uuid(),
            ))
        }))
        .await;

        for (idx, res) in results.into_iter().enumerate() {
            if let Err(e) = res {
                log::error!("Failed to publish subscription event for subscription {idx}: {e}");
            }
        }

        Ok(())
    }
}
