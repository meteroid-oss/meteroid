use super::context::SubscriptionCreationContext;
use super::payment_method::PaymentSetupResult;
use crate::constants::{Currencies, Currency};
use crate::domain::coupons::Coupon;
use crate::domain::enums::SubscriptionEventType;
use crate::domain::slot_transactions::{SlotTransaction, SlotTransactionNewInternal};
use crate::domain::{
    CreateSubscription, CreateSubscriptionAddOns, CreateSubscriptionComponents,
    CreatedSubscription, Customer, SubscriptionActivationCondition, SubscriptionAddOnNew,
    SubscriptionAddOnNewInternal, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionFee, SubscriptionNew, SubscriptionNewEnriched, SubscriptionStatusEnum,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::subscriptions::generate_checkout_token;
use crate::services::InvoiceBillingMode;
use crate::services::subscriptions::utils::{
    apply_coupons, calculate_mrr, extract_billing_period, process_create_subscription_add_ons,
    process_create_subscription_components, process_create_subscription_coupons,
};
use crate::store::PgConn;
use crate::utils::periods::calculate_advance_period_range;
use crate::{StoreResult, services::Services};
use chrono::{Datelike, NaiveDate, NaiveTime};
use common_domain::ids::{BaseId, SlotTransactionId, SubscriptionId, TenantId};
use common_eventbus::{Event, EventBus};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::enums::CycleActionEnum;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use error_stack::{Report, Result, ResultExt};
use futures::TryFutureExt;
use secrecy::SecretString;
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
}

pub struct DetailedSubscription {
    pub subscription: SubscriptionNew,
    components: Vec<SubscriptionComponentNewInternal>,
    add_ons: Vec<SubscriptionAddOnNewInternal>,
    coupons: Vec<Coupon>,
    pub customer: Customer,
    // pub invoicing_entity: InvoicingEntityProviderSensitive,
    currency: Currency,
    pub slot_transactions: Vec<SlotTransactionNewInternal>,
}

impl Services {
    pub(crate) fn build_subscription_details(
        &self,
        batch: &[CreateSubscription],
        context: &SubscriptionCreationContext,
    ) -> StoreResult<Vec<DetailedSubscription>> {
        let res = batch
            .iter()
            .map(|params| {
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
                    .attach_printable("Customer not found")?;

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
                    .attach_printable("Failed to resolve currency")?
                    .clone();

                let components =
                    self.process_components(price_components, subscription, context)?;
                let subscription_add_ons = self.process_add_ons(add_ons, context)?;

                let slot_transactions = process_slot_transactions(
                    &components,
                    &subscription_add_ons,
                    subscription.start_date,
                )?;

                let coupons = if let Some(coupons) = coupons {
                    let coupons = context
                        .all_coupons
                        .iter()
                        .filter(|c| {
                            coupons
                                .coupons
                                .iter()
                                .any(|coupon| c.id == coupon.coupon_id)
                        })
                        .cloned()
                        .collect::<Vec<_>>();

                    coupons
                } else {
                    vec![]
                };

                Ok({
                    DetailedSubscription {
                        subscription: subscription.clone(),
                        components,
                        add_ons: subscription_add_ons,
                        coupons,
                        customer: customer.clone(),
                        currency,
                        slot_transactions,
                    }
                })
            })
            .collect::<Result<Vec<DetailedSubscription>, _>>();

        res
    }

    pub(crate) fn process_subscription(
        &self,
        sub: &DetailedSubscription,
        payment_setup_result: &PaymentSetupResult,
        context: &SubscriptionCreationContext,
        tenant_id: TenantId,
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

        let billing_day_anchor = sub.subscription.billing_day_anchor.unwrap_or_else(|| {
            sub.subscription
                .billing_start_date
                .unwrap_or(sub.subscription.start_date)
                .day() as u16
        });

        let billing_start_date = sub
            .subscription
            .billing_start_date
            .unwrap_or(sub.subscription.start_date);

        let net_terms = sub.subscription.net_terms.unwrap_or(plan.net_terms as u32);

        let activated_at = match sub.subscription.activation_condition {
            SubscriptionActivationCondition::OnStart => {
                sub.subscription.start_date.and_hms_opt(0, 0, 0)
            }
            _ => None,
        };

        let now = chrono::Utc::now().naive_utc();

        // let mut scheduled_event = None;
        let mut current_period_start = billing_start_date;
        let mut current_period_end = None;
        let mut next_cycle_action = None;
        let mut cycle_index = None;
        let mut status = SubscriptionStatusEnum::PendingActivation; // TODO should add pending_checkout ? or we just infer from activation_condition ?

        if sub.subscription.activation_condition == SubscriptionActivationCondition::OnStart {
            if sub.subscription.start_date <= now.date() {
                if sub.subscription.trial_duration.is_some() {
                    status = SubscriptionStatusEnum::TrialActive;
                    current_period_start = billing_start_date;
                    current_period_end = Some(
                        current_period_start
                            + chrono::Duration::days(
                                sub.subscription.trial_duration.unwrap() as i64
                            ),
                    );
                    next_cycle_action = Some(CycleActionEnum::EndTrial);
                } else {
                    let range = calculate_advance_period_range(
                        billing_start_date,
                        billing_day_anchor as u32,
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
        }

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
        };

        let subscription_row = enriched.map_to_row();

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
            .collect::<std::result::Result<Vec<_>, StoreErrorReport>>()?;

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
        })
    }

    fn process_components(
        &self,
        components: &Option<CreateSubscriptionComponents>,
        subscription: &SubscriptionNew,
        context: &SubscriptionCreationContext,
    ) -> Result<Vec<SubscriptionComponentNewInternal>, StoreError> {
        process_create_subscription_components(
            components,
            &context.price_components_by_plan_version,
            subscription,
        )
    }

    fn process_add_ons(
        &self,
        add_ons: &Option<CreateSubscriptionAddOns>,
        context: &SubscriptionCreationContext,
    ) -> Result<Vec<SubscriptionAddOnNewInternal>, StoreError> {
        process_create_subscription_add_ons(add_ons, &context.all_add_ons)
    }

    fn process_coupons(
        &self,
        subscription: &SubscriptionRowNew,
        coupons: &[Coupon],
    ) -> Result<Vec<AppliedCouponRowNew>, StoreError> {
        process_create_subscription_coupons(subscription, coupons)
    }

    fn build_subscription_event(
        &self,
        subscription: &SubscriptionRowNew,
        components: &[SubscriptionComponentNewInternal],
        add_ons: &[SubscriptionAddOnNewInternal],
        _coupons: &[Coupon],
        precision: u8,
    ) -> Result<SubscriptionEventRow, StoreError> {
        let cmrr: i64 = components
            .iter()
            .map(|c| calculate_mrr(&c.fee, &c.period, precision))
            .sum();

        let ao_mrr: i64 = add_ons
            .iter()
            .map(|c| calculate_mrr(&c.fee, &c.period, precision))
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
    components: &Vec<SubscriptionComponentNewInternal>,
    addons: &Vec<SubscriptionAddOnNewInternal>,
    start_date: NaiveDate,
) -> StoreResult<Vec<SlotTransactionNewInternal>> {
    let mut transactions = vec![];

    fn fee_to_tx(
        fee: &SubscriptionFee,
        start_date: NaiveDate,
    ) -> Option<SlotTransactionNewInternal> {
        match &fee {
            SubscriptionFee::Slot {
                initial_slots,
                unit,
                ..
            } => Some(SlotTransactionNewInternal {
                id: SlotTransactionId::new(),
                unit: unit.clone(),
                delta: 0i32,
                prev_active_slots: *initial_slots as i32,
                effective_at: start_date.and_time(NaiveTime::MIN),
                transaction_at: start_date.and_time(NaiveTime::MIN),
            }),
            _ => None,
        }
    }

    for component in components {
        if let Some(tx) = fee_to_tx(&component.fee, start_date) {
            transactions.push(tx)
        }
    }

    for addon in addons {
        if let Some(tx) = fee_to_tx(&addon.fee, start_date) {
            transactions.push(tx)
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
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let res = self
            .store
            .transaction_with(conn, |conn| {
                async move {
                    // Flatten collections for batch insertion
                    let subscriptions: Vec<_> = processed.iter().map(|p| &p.subscription).collect();
                    let components: Vec<_> = processed.iter().flat_map(|p| &p.components).collect();
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

                    apply_coupons(conn, &coupons, &inserted, tenant_id)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    SubscriptionEventRow::insert_batch(conn, events)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    SlotTransactionRow::insert_batch(conn, slot_transactions)
                        .map_err(Into::<StoreErrorReport>::into)
                        .await?;

                    self.insert_created_outbox_events_tx(conn, &inserted, tenant_id)
                        .await?;

                    for sub in &inserted {
                        // Only bill if the subscription is active and not pending checkout/in trial TODO check paid trial
                        if sub.activated_at.is_none()
                            || sub.pending_checkout
                            || sub.trial_duration.is_some()
                        {
                            continue;
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

        let inserted_with_tokens = res
            .into_iter()
            .map(|mut sub| {
                if sub.pending_checkout {
                    sub.checkout_token =
                        Some(generate_checkout_token(jwt_secret, tenant_id, sub.id)?);
                }
                Ok(sub)
            })
            .collect::<StoreResult<Vec<_>>>()?;

        Ok(inserted_with_tokens)
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
                log::error!(
                    "Failed to publish subscription event for subscription {}: {}",
                    idx,
                    e
                );
            }
        }

        Ok(())
    }
}
