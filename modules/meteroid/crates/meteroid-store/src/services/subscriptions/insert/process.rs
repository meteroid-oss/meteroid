use super::context::SubscriptionCreationContext;
use super::payment_method::PaymentSetupResult;
use crate::constants::{Currencies, Currency};
use crate::domain::checkout_sessions::{CheckoutType, CreateCheckoutSession};
use crate::domain::coupons::Coupon;
use crate::domain::custom_property_definitions::CustomPropertyMode;
use crate::domain::entity_activity::Actor;
use crate::domain::enums::{CustomPropertyEntityType, SubscriptionEventType};
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
    PendingMaterialization, apply_coupons, apply_coupons_at, apply_coupons_without_validation,
    creation_minimum_for_mrr, extract_billing_period, process_create_subscription_add_ons,
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
use error_stack::{Report, ResultExt, bail};
use futures::TryFutureExt;
use scoped_futures::ScopedFutureExt;
use secrecy::SecretString;
use std::collections::{HashMap, HashSet};
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
    /// Emitted (in addition to `event`) when the subscription is born already Completed
    /// (a migrated, already-ended subscription) so the analytics running total decrements
    /// it at its end_date. The +1 at start comes from the DB activation trigger.
    ended_event: Option<SubscriptionEventRow>,
    slot_transactions: Vec<SlotTransactionRow>,
    scheduled_events: Vec<ScheduledEventNew>,
    /// Products/prices that need to be created inside the persist transaction (components).
    pending_materializations: Vec<PendingMaterialization>,
    /// Products/prices that need to be created inside the persist transaction (add-ons).
    pending_addon_materializations: Vec<PendingMaterialization>,
    /// When true, skip checkout session creation even if pending_checkout is true.
    /// Used for subscriptions created from checkout completion (SelfServe flow).
    skip_checkout_session: bool,
    /// When true, skip billing for this subscription (migration mode).
    skip_past_invoices: bool,
    entitlements: Vec<crate::domain::entitlements::EntitlementSpec>,
    /// Snapshot of the plan minimum at subscription creation time. None when the plan has no minimum.
    minimum: Option<crate::domain::plan_minimums::PlanMinimum>,
    /// Operator-set minimum override. When present it supersedes `minimum` and is written as
    /// an `is_override = true` row.
    minimum_override: Option<(i64, crate::domain::plan_minimums::CreateMinimumScope)>,
    /// When true (and no override), the plan minimum snapshot is skipped entirely.
    drop_minimum: bool,
    /// Inserted not started: the lifecycle starts it and bills its first period.
    start_deferred: bool,
}

impl ProcessedSubscription {
    /// Inserts it not started whatever its start date, for the lifecycle to start it at its
    /// business time (`ActivateSubscription`): the first period is billed and what is owed at
    /// start is funded then, not when it is inserted.
    pub(crate) fn defer_start(&mut self) -> StoreResult<()> {
        let row = &mut self.subscription;
        if row.activation_condition
            != diesel_models::enums::SubscriptionActivationConditionEnum::OnStart
            || row.trial_duration.is_some()
            || row.imported_at.is_some()
            || row.pending_checkout
            || !self.scheduled_events.is_empty()
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "only a subscription that starts on its start date, without a trial, can be \
                 started later"
                    .to_string(),
            )));
        }
        row.status = diesel_models::enums::SubscriptionStatusEnum::PendingActivation;
        row.current_period_start = row.billing_start_date.unwrap_or(row.start_date);
        row.current_period_end = Some(row.start_date);
        row.next_cycle_action = Some(CycleActionEnum::ActivateSubscription);
        row.cycle_index = None;
        self.start_deferred = true;
        Ok(())
    }
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
    pub pending_addon_materializations: Vec<PendingMaterialization>,
    pub entitlements: Vec<crate::domain::entitlements::EntitlementSpec>,
    /// Operator-set minimum override applied at creation. Replaces the plan snapshot when present.
    minimum_override: Option<(i64, crate::domain::plan_minimums::CreateMinimumScope)>,
    /// When true (and no override), the plan minimum snapshot is skipped entirely.
    drop_minimum: bool,
    /// The tax the quoted prices carry, resolved in the gather step. `None` for a
    /// tax-exclusive plan or an untaxed customer.
    tax_inclusive_rate: Option<rust_decimal::Decimal>,
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
                entitlements,
                minimum_override,
                drop_minimum,
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
            let (subscription_add_ons, pending_addon_materializations) =
                self.process_add_ons(add_ons, subscription, context, plan)?;

            let slot_transactions = process_slot_transactions(
                &components,
                &subscription_add_ons,
                subscription.start_date,
            )?;

            let coupons_resolved = if let Some(coupons) = coupons {
                // Preserve request order so applied_coupon UUIDs (and thus discount
                // application order) match the order the caller specified.
                coupons
                    .coupons
                    .iter()
                    .filter_map(|req| {
                        context
                            .all_coupons
                            .iter()
                            .find(|c| c.id == req.coupon_id)
                            .cloned()
                    })
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
                pending_addon_materializations,
                entitlements: entitlements.clone(),
                minimum_override: minimum_override.clone(),
                drop_minimum: *drop_minimum,
                tax_inclusive_rate: context.tax_inclusive_rates.get(idx).copied().flatten(),
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

        let coupons: Vec<Coupon> = params
            .coupon_ids
            .iter()
            .filter_map(|id| context.all_coupons.iter().find(|c| c.id == *id).cloned())
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
            pending_addon_materializations: vec![], // Quotes have pre-resolved add-on prices
            entitlements: params.entitlements.clone(),
            minimum_override: params.minimum_override.clone(),
            drop_minimum: false,
            tax_inclusive_rate: context.tax_inclusive_rates.first().copied().flatten(),
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

        // Validate: end_date must not be before start_date
        if let Some(end_date) = sub.subscription.end_date
            && end_date < sub.subscription.start_date
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "end_date must not be before start_date".to_string(),
            )));
        }

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

        let sells_credit = sub.components.iter().any(|c| c.fee.sells_credit())
            || sub.add_ons.iter().any(|a| a.fee.sells_credit());
        let net_terms = sub.subscription.net_terms.unwrap_or_else(|| {
            crate::domain::subscriptions::default_net_terms(
                sells_credit,
                u32::try_from(plan.net_terms).unwrap_or(0),
            )
        });

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

                    if let Some(end_date) = sub.subscription.end_date
                        && end_date < now.date()
                    {
                        // end_date is already in the past: complete the subscription immediately.
                        // Checked before trial state — a subscription past its end_date is always
                        // Completed, regardless of whether a trial would otherwise still be active.
                        // No cycle worker involvement needed — billing was intentionally skipped
                        // (skip_past_invoices = true), and there is no current period to charge.
                        let last_period = find_period_containing_date(
                            effective_billing_start,
                            end_date,
                            &period,
                            billing_day_anchor as u32,
                        );
                        let elapsed = calculate_elapsed_cycles(
                            effective_billing_start,
                            end_date,
                            &period,
                            billing_day_anchor as u32,
                        );
                        status = SubscriptionStatusEnum::Completed;
                        cycle_index = Some(elapsed);
                        current_period_start = last_period.start;
                        current_period_end = Some(end_date);
                        next_cycle_action = None;
                    } else if trial_still_active {
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
            tax_inclusive_rate: sub.tax_inclusive_rate,
        };

        let subscription_row = enriched.map_to_row()?;

        let subscription_coupons = self.process_coupons(&subscription_row, &sub.coupons)?;

        let minimum_for_mrr = creation_minimum_for_mrr(
            &sub.components,
            context
                .minimums_by_plan_version
                .get(&sub.subscription.plan_version_id),
            sub.minimum_override.as_ref(),
            sub.drop_minimum,
        );

        let event = self.build_subscription_event(
            &subscription_row,
            &sub.components,
            &sub.add_ons,
            &context.all_coupons,
            sub.currency.precision,
            minimum_for_mrr.as_ref(),
            period,
        )?;

        // A subscription born already Completed (migration of an already-ended subscription) is
        // stamped activated_at = start_date (trigger emits +1 at start). Emit the matching Ended
        // (-1) at its end_date for the active-count curve. mrr_delta stays None: a migrated sub
        // skips invoices, so its Created event booked no MRR movement to churn.
        let ended_event = (subscription_row.status
            == diesel_models::enums::SubscriptionStatusEnum::Completed)
            .then(|| {
                let applies_to = subscription_row
                    .end_date
                    .or(subscription_row.current_period_end)
                    .unwrap_or(subscription_row.start_date);
                SubscriptionEventRow::ended(subscription_row.id, applies_to, None)
            });

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

        let minimum = context
            .minimums_by_plan_version
            .get(&sub.subscription.plan_version_id)
            .cloned();

        Ok(ProcessedSubscription {
            subscription: subscription_row,
            components,
            add_ons: subscription_add_ons,
            coupons: subscription_coupons,
            event,
            ended_event,
            slot_transactions,
            scheduled_events,
            pending_materializations: sub.pending_materializations.clone(),
            pending_addon_materializations: sub.pending_addon_materializations.clone(),
            skip_checkout_session: sub.subscription.skip_checkout_session,
            skip_past_invoices: sub.subscription.skip_past_invoices,
            entitlements: sub.entitlements.clone(),
            minimum,
            minimum_override: sub.minimum_override.clone(),
            drop_minimum: sub.drop_minimum,
            start_deferred: false,
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
        let effective_from = subscription
            .billing_start_date
            .unwrap_or(subscription.start_date);

        process_create_subscription_components(
            components,
            &context.price_components_by_plan_version,
            subscription,
            &context.products_by_id,
            &context.credit_units,
            resolved,
            plan.product_family_id,
            &plan.currency,
            effective_from,
        )
    }

    fn process_add_ons(
        &self,
        add_ons: &Option<CreateSubscriptionAddOns>,
        subscription: &SubscriptionNew,
        context: &SubscriptionCreationContext,
        plan: &crate::domain::PlanForSubscription,
    ) -> Result<
        (
            Vec<SubscriptionAddOnNewInternal>,
            Vec<PendingMaterialization>,
        ),
        StoreErrorReport,
    > {
        let effective_from = subscription
            .billing_start_date
            .unwrap_or(subscription.start_date);

        process_create_subscription_add_ons(
            add_ons,
            &context.all_add_ons,
            &context.products_by_id,
            &context.addon_prices_by_id,
            &context.credit_units,
            plan.product_family_id,
            &plan.currency,
            effective_from,
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
        minimum: Option<&(i64, crate::domain::plan_minimums::MinimumScope)>,
        period: crate::domain::enums::BillingPeriodEnum,
    ) -> Result<SubscriptionEventRow, StoreErrorReport> {
        // Floor the MRR by the minimum commitment: a subscription whose line spend is below its
        // committed floor still recognizes the floor as recurring revenue (e.g. rate 80 / minimum
        // 90 ⇒ MRR 90), mirroring the invoice shortfall line.
        let mrr_delta =
            crate::services::subscriptions::utils::calculate_subscription_mrr_with_floor(
                components,
                add_ons,
                minimum,
                period,
                precision,
                subscription.tax_inclusive_rate,
            );

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

fn filter_of(row: &SubscriptionRowNew) -> StoreResult<Option<crate::domain::UsageFilter>> {
    crate::domain::UsageFilter::from_columns(
        row.usage_filter_property.clone(),
        row.usage_filter_value.clone(),
    )?
    .map(|filter| crate::domain::UsageFilter::new(filter.property, filter.value))
    .transpose()
}

/// A usage filter is unique per customer over overlapping lifetimes, among the new subscriptions
/// as against the existing ones, checked under the customers' locks (the caller takes them).
async fn ensure_usage_filters_available(
    conn: &mut PgConn,
    tenant_id: TenantId,
    rows: &[SubscriptionRowNew],
) -> StoreResult<()> {
    use crate::repositories::subscriptions::usage_attribution::{
        ensure_usage_filter_available, midnight,
    };

    let mut filtered = Vec::new();
    for row in rows {
        if let Some(filter) = filter_of(row)? {
            filtered.push((row, filter));
        }
    }

    for (index, (row, filter)) in filtered.iter().enumerate() {
        let overlapping = filtered[..index].iter().any(|(other, other_filter)| {
            other.customer_id == row.customer_id
                && other_filter == filter
                && other.end_date.is_none_or(|end| row.start_date < end)
                && row
                    .end_date
                    .is_none_or(|row_end| other.start_date < row_end)
        });
        if overlapping {
            bail!(StoreError::InvalidArgument(format!(
                "two new subscriptions of a customer bill the events where {} is {}",
                filter.property, filter.value
            )));
        }
        ensure_usage_filter_available(
            conn,
            tenant_id,
            row.customer_id,
            row.id,
            filter,
            midnight(row.start_date),
            row.end_date.map(midnight),
        )
        .await?;
    }
    Ok(())
}

/// Records the usage filter of each new subscription from its start, refusing one that would
/// move events another subscription of the customer has already invoiced.
async fn record_created_usage_filters(
    conn: &mut PgConn,
    tenant_id: TenantId,
    rows: &[SubscriptionRowNew],
) -> StoreResult<()> {
    use crate::repositories::subscriptions::usage_attribution::{
        midnight, record_usage_filter_version, usage_filter_effective_from,
    };

    let created = rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    for row in rows {
        let Some(filter) = filter_of(row)? else {
            continue;
        };
        let stored = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, row.id)
            .await
            .map_err(Into::<StoreErrorReport>::into)?
            .subscription;
        let start = midnight(row.start_date);
        let from = usage_filter_effective_from(
            conn,
            tenant_id,
            &SubscriptionRow {
                usage_filter_property: None,
                usage_filter_value: None,
                ..stored
            },
            start,
            Some(&filter),
            &created,
        )
        .await?;
        if from > start {
            // A start date is a day: the first one whose midnight is not before `from`.
            let earliest = if from.time() == chrono::NaiveTime::MIN {
                from.date()
            } else {
                from.date() + chrono::Days::new(1)
            };
            bail!(StoreError::InvalidArgument(format!(
                "a usage filter cannot apply before {}: other subscriptions of this customer have \
                 invoiced the usage it would claim up to then. Start the subscription on or after \
                 that date, or create it without a usage filter",
                earliest.format("%b %-d, %Y")
            )));
        }
        record_usage_filter_version(conn, tenant_id, row.id, Some(&filter), start).await?;
    }
    Ok(())
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
        actor: &Actor,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        self.persist_subscriptions_internal(
            conn, actor, processed, tenant_id, jwt_secret, public_url, false, None,
        )
        .await
    }

    /// Persist subscriptions without coupon validation.
    ///
    /// Use this for async payment flows where coupons were already validated before charging.
    pub(crate) async fn persist_subscriptions_skip_coupon_validation(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        self.persist_subscriptions_internal(
            conn, actor, processed, tenant_id, jwt_secret, public_url, true, None,
        )
        .await
    }

    /// `persist_subscriptions` whose coupons are checked and redeemed as of `redeemed_at`, a
    /// sandbox replay's start of the subscriptions.
    pub(crate) async fn persist_subscriptions_at(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
        redeemed_at: chrono::DateTime<chrono::Utc>,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        self.persist_subscriptions_internal(
            conn,
            actor,
            processed,
            tenant_id,
            jwt_secret,
            public_url,
            false,
            Some(redeemed_at),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_subscriptions_internal(
        &self,
        conn: &mut PgConn,
        actor: &Actor,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        public_url: &String,
        skip_coupon_validation: bool,
        redeemed_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        // A subscription billed at once is finalized under its customer's lock: its metering is
        // read first (outermost when inserted on a connection of its own).
        let res = self
            .transaction_reading_usage_first(conn, |services, conn| {
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
                                mat.product_family_id,
                                &mat.currency,
                                false,
                            )
                            .await?;

                            materialized.insert(
                                (sub_idx, mat.component_index),
                                (product_id, price_ids.into_iter().next()),
                            );
                        }
                    }

                    // Materialize pending add-on products/prices inside the transaction.
                    let mut materialized_addons: HashMap<
                        (usize, usize),
                        (
                            common_domain::ids::ProductId,
                            Option<common_domain::ids::PriceId>,
                        ),
                    > = HashMap::new();
                    for (sub_idx, proc) in processed.iter().enumerate() {
                        for mat in &proc.pending_addon_materializations {
                            use crate::domain::price_components::PriceComponentNewInternal;
                            use crate::repositories::price_components::resolve_component_internal;

                            let internal = PriceComponentNewInternal {
                                name: mat.name.clone(),
                                product_ref: mat.product_ref.clone(),
                                prices: vec![mat.price_entry.clone()],
                            };
                            let (_product_id, price_ids) = resolve_component_internal(
                                conn,
                                &internal,
                                tenant_id,
                                mat.product_family_id,
                                &mat.currency,
                                false,
                            )
                            .await?;

                            materialized_addons.insert(
                                (sub_idx, mat.component_index),
                                (_product_id, price_ids.into_iter().next()),
                            );
                        }
                    }

                    // Flatten collections for batch insertion, patching materialized components.
                    // Validate/normalize any user-supplied custom properties against the tenant's
                    // subscription definitions; a null bag (system/import flows) falls back to {}.
                    let mut owned_subscriptions: Vec<SubscriptionRowNew> =
                        Vec::with_capacity(processed.len());
                    for p in processed.iter() {
                        let mut row = p.subscription.clone();
                        row.custom_properties = self
                            .store
                            .internal
                            .validate_and_normalize_custom_properties(
                                conn,
                                tenant_id,
                                CustomPropertyEntityType::Subscription,
                                &row.custom_properties,
                                &serde_json::json!({}),
                                CustomPropertyMode::Replace,
                            )
                            .await?;
                        owned_subscriptions.push(row);
                    }
                    // Every customer of the batch first, in id order: the usage filter checks, the
                    // coupons (tenant rows) and the billing below all come after it.
                    let mut customers = owned_subscriptions
                        .iter()
                        .map(|row| row.customer_id)
                        .collect::<Vec<_>>();
                    customers.sort_by_key(|id| id.as_uuid());
                    customers.dedup();
                    let mut locks = Vec::with_capacity(customers.len());
                    for customer_id in customers {
                        locks.push(
                            crate::repositories::credit::customer_credit::lock_customer_tx(
                                conn,
                                tenant_id,
                                customer_id,
                            )
                            .await?,
                        );
                    }
                    diesel_models::query::credit_inputs::bump_customers(
                        conn,
                        &locks.iter().collect::<Vec<_>>(),
                    )
                    .await?;
                    ensure_usage_filters_available(conn, tenant_id, &owned_subscriptions).await?;
                    let subscriptions: Vec<_> = owned_subscriptions.iter().collect();

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

                    // Patch materialized add-on rows
                    let mut patched_add_ons: Vec<SubscriptionAddOnRowNew> = Vec::new();
                    for (sub_idx, proc) in processed.iter().enumerate() {
                        for (addon_idx, add_on) in proc.add_ons.iter().enumerate() {
                            if let Some((_product_id, price_id)) =
                                materialized_addons.get(&(sub_idx, addon_idx))
                            {
                                let mut patched = add_on.clone();
                                patched.price_id = *price_id;
                                patched.legacy_fee = None;
                                patched_add_ons.push(patched);
                            } else {
                                patched_add_ons.push(add_on.clone());
                            }
                        }
                    }
                    let add_ons: Vec<_> = patched_add_ons.iter().collect();
                    let coupons: Vec<_> = processed.iter().flat_map(|p| &p.coupons).collect();
                    let events: Vec<_> = processed
                        .iter()
                        .flat_map(|p| std::iter::once(&p.event).chain(p.ended_event.iter()))
                        .collect();
                    let slot_transactions: Vec<_> = processed
                        .iter()
                        .flat_map(|p| &p.slot_transactions)
                        .collect();

                    // Perform batch insertions
                    let inserted: Vec<CreatedSubscription> =
                        SubscriptionRow::insert_subscription_batch(conn, subscriptions)
                            .await
                            .map(|v| v.into_iter().map(Into::into).collect())?;

                    for (proc, created) in processed.iter().zip(inserted.iter()) {
                        if !proc.entitlements.is_empty() {
                            crate::repositories::entitlements::insert_entitlement_specs(
                                conn,
                                proc.entitlements.clone(),
                                common_domain::ids::EntitlementEntityId::Subscription(created.id),
                                tenant_id,
                            )
                            .await?;
                        }
                    }

                    SubscriptionComponentRow::insert_subscription_component_batch(conn, components)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    // Snapshot plan minimums onto subscriptions (mirrors the component snapshot).
                    {
                        use diesel_models::subscription_adjustments::{
                            SubscriptionAdjustmentComponentRowNew, SubscriptionAdjustmentRowNew,
                        };

                        let mut minimum_rows: Vec<SubscriptionAdjustmentRowNew> = Vec::new();
                        let mut minimum_component_rows: Vec<SubscriptionAdjustmentComponentRowNew> =
                            Vec::new();
                        for (sub_idx, proc) in processed.iter().enumerate() {
                            // A creation-time override supersedes the plan snapshot entirely;
                            // an explicit drop suppresses the plan snapshot (no floor at all).
                            //
                            // The override scope is unresolved: it may name inline new-product
                            // extras whose product ids only exist after materialization above.
                            // Resolve them here against this subscription's components (existing
                            // product ids + the freshly-minted ones in `materialized`).
                            let effective: Option<(
                                i64,
                                crate::domain::plan_minimums::MinimumScope,
                                bool,
                            )> = match &proc.minimum_override {
                                Some((amount_subunit, create_scope)) => {
                                    let scope = if create_scope.is_all_components() {
                                        crate::domain::plan_minimums::MinimumScope::AllComponents
                                    } else {
                                        // (name, product_id) for this sub's components (existing
                                        // product ids + the freshly-minted ones in `materialized`).
                                        // resolve() rejects names that are ambiguous across them.
                                        let named_products: Vec<(
                                            String,
                                            common_domain::ids::ProductId,
                                        )> = proc
                                            .components
                                            .iter()
                                            .enumerate()
                                            .filter_map(|(comp_idx, comp)| {
                                                materialized
                                                    .get(&(sub_idx, comp_idx))
                                                    .map(|(p, _)| *p)
                                                    .or(comp.product_id)
                                                    .map(|pid| (comp.name.clone(), pid))
                                            })
                                            .collect();
                                        create_scope.resolve(&named_products).map_err(|e| {
                                            Report::new(StoreError::InvalidArgument(e))
                                        })?
                                    };
                                    Some((*amount_subunit, scope, true))
                                }
                                None if proc.drop_minimum => None,
                                None => proc
                                    .minimum
                                    .as_ref()
                                    .map(|pm| (pm.amount_subunit, pm.scope.clone(), false)),
                            };

                            if let Some((amount_subunit, scope, is_override)) = effective {
                                let subscription_id = proc.subscription.id;
                                minimum_rows.push(SubscriptionAdjustmentRowNew {
                                    subscription_id,
                                    kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                                    amount_subunit,
                                    includes_all_components: matches!(
                                        scope,
                                        crate::domain::plan_minimums::MinimumScope::AllComponents
                                    ),
                                    is_override,
                                });
                                if let crate::domain::plan_minimums::MinimumScope::Products(ids) =
                                    &scope
                                {
                                    for &product_id in ids {
                                        minimum_component_rows.push(
                                            SubscriptionAdjustmentComponentRowNew {
                                                subscription_id,
                                                kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
                                                product_id,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        use diesel_models::subscription_adjustments::SubscriptionAdjustmentComponentRow;
                        SubscriptionAdjustmentRowNew::insert_batch(conn, &minimum_rows)
                            .map_err(Into::<StoreErrorReport>::into)
                            .await?;
                        SubscriptionAdjustmentComponentRow::insert_batch(
                            conn,
                            &minimum_component_rows,
                        )
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;
                    }

                    SubscriptionAddOnRow::insert_batch(conn, add_ons)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    record_created_usage_filters(conn, tenant_id, &owned_subscriptions).await?;

                    if skip_coupon_validation {
                        apply_coupons_without_validation(conn, &*services.store.hooks.credit, &coupons, tenant_id).await?;
                    } else {
                        match redeemed_at {
                            Some(at) => {
                                apply_coupons_at(conn, &*services.store.hooks.credit, &coupons, &inserted, tenant_id, at).await?
                            }
                            None => apply_coupons(conn, &*services.store.hooks.credit, &coupons, &inserted, tenant_id).await?,
                        };
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

                    services.insert_created_outbox_events_tx(conn, actor, &inserted, tenant_id)
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
                                change_date: None,
                                success_url: None,
                                cancel_url: None,
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
                        if proc.skip_past_invoices || proc.start_deferred {
                            continue;
                        }

                        // A subscription starting later is invoiced when the lifecycle starts it,
                        // on its start date (Stripe subscription schedules, Lago pending).
                        if sub.status == SubscriptionStatusEnum::PendingActivation {
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

                        services.bill_subscription_tx(
                            conn,
                            actor,
                            tenant_id,
                            sub.id,
                            InvoiceBillingMode::Immediate,
                        )
                        .await?;
                        services.book_created_mrr_tx(conn, tenant_id, sub.id).await?;
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
            if sub.pending_checkout
                && let Some(session) =
                    CheckoutSessionRow::get_by_subscription(conn, tenant_id, sub.id)
                        .await
                        .ok()
                        .flatten()
            {
                let token = generate_portal_token(
                    jwt_secret,
                    tenant_id,
                    ResourceAccess::CheckoutSession(session.id),
                    None,
                    None,
                    false,
                )?;
                let checkout_url = format!("{}/checkout?token={}", public_url, token);
                sub.checkout_url = Some(checkout_url);
            }
            inserted_with_checkout_urls.push(sub);
        }

        Ok(inserted_with_checkout_urls)
    }

    pub async fn handle_post_insertion(
        &self,
        actor: &Actor,
        event_bus: Arc<dyn EventBus<Event>>,
        inserted: &[CreatedSubscription],
    ) -> StoreResult<()> {
        // Publish events
        self.publish_subscription_events(actor, event_bus, inserted)
            .await?;

        Ok(())
    }

    async fn publish_subscription_events(
        &self,
        actor: &Actor,
        event_bus: Arc<dyn EventBus<Event>>,
        subscriptions: &[CreatedSubscription],
    ) -> StoreResult<()> {
        let results = futures::future::join_all(subscriptions.iter().map(|sub| {
            event_bus.publish(Event::subscription_created(
                actor.clone(),
                sub.id,
                sub.tenant_id,
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
