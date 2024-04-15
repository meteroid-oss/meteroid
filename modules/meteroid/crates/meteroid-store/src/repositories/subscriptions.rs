use crate::domain::enums::{BillingPeriodEnum, SubscriptionEventType};
use crate::domain::{
    CreateSubscriptionComponents, FeeType, PaginatedVec, PaginationRequest, Subscription,
    SubscriptionComponent, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionDetails, SubscriptionFee, SubscriptionNew,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use crate::{domain, StoreResult};
use chrono::NaiveDate;
use common_utils::decimal::ToCent;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_models::errors::DatabaseErrorContainer;
use error_stack::Report;
use std::collections::HashMap;
use uuid::Uuid;

use rust_decimal::prelude::*;

use itertools::Itertools;

pub enum CancellationEffectiveAt {
    EndOfBillingPeriod,
}

#[async_trait::async_trait]
pub trait SubscriptionInterface {
    async fn insert_subscription(
        &self,
        subscription: domain::CreateSubscription,
    ) -> StoreResult<domain::CreatedSubscription>;

    async fn insert_subscription_batch(
        &self,
        batch: Vec<domain::CreateSubscription>,
    ) -> StoreResult<Vec<domain::CreatedSubscription>>;

    async fn get_subscription_details(
        &self,
        tenant_id: Uuid,
        subscription_id: Uuid,
    ) -> StoreResult<domain::SubscriptionDetails>;

    // async fn get_subscription(
    //     &self,
    //     subscription_id: Uuid,
    // ) -> StoreResult<domain::Subscription>;

    async fn insert_subscription_components(
        &self,
        tenant_id: Uuid,
        batch: Vec<domain::SubscriptionComponentNew>,
    ) -> StoreResult<Vec<domain::SubscriptionComponent>>;

    async fn cancel_subscription(
        &self,
        subscription_id: Uuid,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        context: domain::TenantContext,
    ) -> StoreResult<domain::Subscription>;

    async fn list_subscriptions(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
        plan_id: Option<Uuid>,
        pagination: domain::PaginationRequest,
    ) -> StoreResult<domain::PaginatedVec<domain::Subscription>>;
}

// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant

fn calculate_mrr_for_new_component(component: &domain::SubscriptionComponentNewInternal) -> i64 {
    let mut mrr_cents = 0;

    let period_as_months = component.period.as_months() as i64;

    match &component.fee {
        SubscriptionFee::Rate { rate } => {
            mrr_cents = rate.to_cents().unwrap_or(0) * period_as_months;
        }
        SubscriptionFee::Recurring { quantity, rate, .. } => {
            let total = rate * Decimal::from(*quantity);
            mrr_cents = total.to_cents().unwrap_or(0) * period_as_months;
        }
        SubscriptionFee::Capacity { rate, .. } => {
            mrr_cents = rate.to_cents().unwrap_or(0) * period_as_months;
        }
        SubscriptionFee::Slot {
            initial_slots,
            unit_rate,
            ..
        } => {
            mrr_cents =
                (*initial_slots as i64) * unit_rate.to_cents().unwrap_or(0) * period_as_months;
        }
        SubscriptionFee::OneTime { .. } | SubscriptionFee::Usage { .. } => {
            // doesn't count as mrr
        }
    }

    mrr_cents
}

#[async_trait::async_trait]
pub trait SubscriptionSlotsInterface {
    async fn get_current_slots_value(
        &self,
        tenant_id: Uuid,
        subscription_id: Uuid,
        price_component_id: Uuid,
        ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32>;

    async fn add_slot_transaction(
        &self,
        tenant_id: Uuid,
        subscription_id: Uuid,
        price_component_id: Uuid,
        slots: i32,
    ) -> StoreResult<i32>;
}

#[async_trait::async_trait]
impl SubscriptionSlotsInterface for Store {
    async fn get_current_slots_value(
        &self,
        _tenant_id: Uuid,
        subscription_id: Uuid,
        price_component_id: Uuid,
        ts: Option<chrono::NaiveDateTime>,
    ) -> StoreResult<u32> {
        let mut conn = self.get_conn().await?;

        diesel_models::slot_transactions::SlotTransaction::fetch_by_subscription_id_and_price_component_id(
            &mut conn,
            subscription_id,
            price_component_id,
            ts,
        )
            .await
            .map(|c| c.current_active_slots as u32)
            .map_err(Into::into)
    }

    async fn add_slot_transaction(
        &self,
        _tenant_id: Uuid,
        _subscription_id: Uuid,
        _price_component_id: Uuid,
        _slots: i32,
    ) -> StoreResult<i32> {
        todo!()
        /*
        sequenceDiagram
            participant User
            participant Billing Software
            participant Database
            participant Stripe API

            User->>Billing Software: Request to add seats
            Billing Software->>Database: Check current subscription details
            Database-->>Billing Software: Return subscription details
            Billing Software->>Billing Software: Calculate prorated amount for additional seats
            Billing Software->>User: Display prorated charge and request payment approval

            User->>Billing Software: Approve payment
            Billing Software->>Stripe API: Create payment intent with prorated amount
            Stripe API-->>Billing Software: Payment intent created (awaiting confirmation)

            User->>Stripe API: Confirm and process payment
            Stripe API-->>Billing Software: Payment success notification
            Billing Software->>Database: Update subscription (add seats)
            Database-->>Billing Software: Subscription updated
            Billing Software->>Stripe API: Generate invoice for the transaction
            Stripe API-->>Billing Software: Invoice generated
            Billing Software->>User: Confirm seat addition and send invoice

            Billing Software->>Database: Log transaction details
            Database-->>Billing Software: Transaction logged
            Billing Software->>User: Notify transaction completion

                 */
    }
}

#[async_trait::async_trait]
impl SubscriptionInterface for Store {
    async fn insert_subscription(
        &self,
        params: domain::CreateSubscription,
    ) -> StoreResult<domain::CreatedSubscription> {
        self.insert_subscription_batch(vec![params])
            .await?
            .pop()
            .ok_or(StoreError::InsertError.into())
    }

    async fn insert_subscription_batch(
        &self,
        batch: Vec<domain::CreateSubscription>,
    ) -> StoreResult<Vec<domain::CreatedSubscription>> {
        let mut conn: PgConn = self.get_conn().await?;

        struct DieselModelWrapper {
            subscription: diesel_models::subscriptions::SubscriptionNew,
            price_components: Vec<diesel_models::subscription_components::SubscriptionComponentNew>,
            event: diesel_models::subscription_events::SubscriptionEvent,
        }

        let db_price_components_by_plan_version =
            diesel_models::price_components::PriceComponent::get_by_plan_ids(
                &mut conn,
                &batch
                    .iter()
                    .map(|c| c.subscription.plan_version_id)
                    .collect::<Vec<_>>(),
            )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        // map the price compoennts thanks to .try_into
        let price_components_by_plan_version: HashMap<Uuid, Vec<domain::PriceComponent>> =
            db_price_components_by_plan_version
                .into_iter()
                .map(|(k, v)| {
                    let converted_vec: Result<Vec<domain::PriceComponent>, _> =
                        v.into_iter().map(|c| c.try_into()).collect();
                    converted_vec.map(|vec| (k, vec))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;

        let insertable = batch
            .into_iter()
            .map(|params| {
                let domain::CreateSubscription {
                    subscription,
                    price_components,
                } = params;

                // first we need to process the CreateSubscriptionComponents into Vec<SubscriptionComponentNew>
                // we will need the plan version components

                let insertable_subscription_components = process_create_subscription_components(
                    &price_components,
                    &price_components_by_plan_version,
                    &subscription,
                )?;

                let insertable_subscription: diesel_models::subscriptions::SubscriptionNew =
                    subscription.into();

                let cmrr = insertable_subscription_components
                    .iter()
                    .map(|c| calculate_mrr_for_new_component(c))
                    .sum::<i64>();

                let insertable_subscription_components = insertable_subscription_components
                    .into_iter()
                    .map(|c| domain::SubscriptionComponentNew {
                        subscription_id: insertable_subscription.id,
                        internal: c,
                    })
                    .collect::<Vec<_>>();

                let insertable_event = diesel_models::subscription_events::SubscriptionEvent {
                    id: Uuid::now_v7(),
                    subscription_id: insertable_subscription.id,
                    event_type: SubscriptionEventType::Created.into(),
                    details: None,
                    created_at: chrono::Utc::now().naive_utc(),
                    mrr_delta: Some(cmrr),
                    bi_mrr_movement_log_id: None,
                    applies_to: insertable_subscription.billing_start_date,
                };

                Ok::<_, StoreError>(DieselModelWrapper {
                    subscription: insertable_subscription,
                    price_components: insertable_subscription_components
                        .into_iter()
                        .map(|c| c.try_into())
                        .collect::<Result<Vec<_>, _>>()?,
                    event: insertable_event,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let insertable_subscription_components = insertable
            .iter()
            .flat_map(|c| c.price_components.iter())
            .collect::<Vec<_>>();

        let insertable_subscriptions = insertable.iter().map(|c| &c.subscription).collect();

        let insertable_subscription_events: Vec<
            &diesel_models::subscription_events::SubscriptionEvent,
        > = insertable.iter().map(|c| &c.event).collect();

        let inserted_subscriptions = conn.transaction(|conn| async move {
            let inserted_subscriptions: Vec<domain::CreatedSubscription> =
                diesel_models::subscriptions::Subscription::insert_subscription_batch(
                    conn,
                    insertable_subscriptions,
                )
                    .await
                    .map_err(Into::<DatabaseErrorContainer>::into)
                    .map(|v| v.into_iter().map(Into::into).collect())?;


            diesel_models::subscription_components::SubscriptionComponent::insert_subscription_component_batch(
                conn,
                insertable_subscription_components,
            )
                .await
                .map_err(Into::<DatabaseErrorContainer>::into)?;

            diesel_models::subscription_events::SubscriptionEvent::insert_batch(
                conn,
                insertable_subscription_events,
            )
                .await
                .map_err(Into::<DatabaseErrorContainer>::into)?;

            Ok::<_, DatabaseErrorContainer>(inserted_subscriptions)
        }.scope_boxed()).await?;

        Ok(inserted_subscriptions)
    }

    async fn get_subscription_details(
        &self,
        tenant_id: Uuid,
        subscription_id: Uuid,
    ) -> StoreResult<domain::SubscriptionDetails> {
        let mut conn = self.get_conn().await?;

        let db_subscription = diesel_models::subscriptions::Subscription::get_subscription_by_id(
            &mut conn,
            &tenant_id,
            &subscription_id,
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let subscription: domain::Subscription = db_subscription.into();

        let schedules: Vec<domain::Schedule> =
            diesel_models::schedules::Schedule::list_schedules_by_subscription(
                &mut conn,
                &tenant_id,
                &subscription_id,
            )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.into())
                .collect();

        let subscription_components: Vec<domain::SubscriptionComponent> =
            diesel_models::subscription_components::SubscriptionComponent::list_subscription_components_by_subscription(
                &mut conn,
                &tenant_id,
                &subscription_id,
            )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?;

        let metric_ids = subscription_components
            .iter()
            .filter_map(|sc| sc.metric_id())
            .collect::<Vec<_>>();

        let billable_metrics: Vec<domain::BillableMetric> =
            diesel_models::billable_metrics::BillableMetric::get_by_ids(
                &mut conn,
                &metric_ids,
                &subscription.tenant_id,
            )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|m| m.into())
                .collect();

        Ok(domain::SubscriptionDetails {
            id: subscription.id,
            tenant_id: subscription.tenant_id,
            customer_id: subscription.customer_id,
            plan_version_id: subscription.plan_version_id,
            customer_external_id: subscription.customer_alias,
            billing_start_date: subscription.billing_start_date,
            billing_end_date: subscription.billing_end_date,
            billing_day: subscription.billing_day,
            currency: subscription.currency,
            net_terms: subscription.net_terms,
            price_components: subscription_components,
            metrics: billable_metrics,
            mrr_cents: subscription.mrr_cents,
            version: subscription.version,
            plan_name: subscription.plan_name,
            plan_id: subscription.plan_id,
            customer_name: subscription.customer_name,
            schedules,
            canceled_at: subscription.canceled_at,
            invoice_memo: subscription.invoice_memo,
            invoice_threshold: subscription.invoice_threshold,
            created_at: subscription.created_at,
            cancellation_reason: subscription.cancellation_reason,
            activated_at: subscription.activated_at,
            created_by: subscription.created_by,
            trial_start_date: subscription.trial_start_date,
        })
    }

    async fn insert_subscription_components(
        &self,
        _tenant_id: Uuid,
        batch: Vec<SubscriptionComponentNew>,
    ) -> StoreResult<Vec<SubscriptionComponent>> {
        let mut conn = self.get_conn().await?;

        // TODO update mrr

        let insertable_batch: Vec<
            diesel_models::subscription_components::SubscriptionComponentNew,
        > = batch
            .into_iter()
            .map(|c| c.try_into())
            .collect::<Result<Vec<_>, _>>()?;

        diesel_models::subscription_components::SubscriptionComponent::insert_subscription_component_batch(
            &mut conn,
            insertable_batch.iter().collect(),
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|v| v.into_iter()
                .map(|e| e.try_into().map_err(Report::from)).collect::<Result<Vec<_>, _>>())?
    }

    async fn cancel_subscription(
        &self,
        subscription_id: Uuid,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        context: domain::TenantContext,
    ) -> StoreResult<domain::Subscription> {
        let db_subscription = self
            .transaction(|conn| {
                async move {
                    let subscription: SubscriptionDetails = self
                        .get_subscription_details(context.tenant_id, subscription_id)
                        .await?;

                    let now = chrono::Utc::now().naive_utc();

                    let billing_end_date: NaiveDate = match effective_at {
                        CancellationEffectiveAt::EndOfBillingPeriod => subscription
                            .calculate_cancellable_end_of_period_date(now.date())
                            .ok_or(Report::from(StoreError::CancellationError))?,
                    };

                    diesel_models::subscriptions::Subscription::cancel_subscription(
                        conn,
                        diesel_models::subscriptions::CancelSubscriptionParams {
                            subscription_id: subscription_id.clone(),
                            tenant_id: context.tenant_id,
                            billing_end_date,
                            canceled_at: now,
                            reason,
                        },
                    )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let res = diesel_models::subscriptions::Subscription::get_subscription_by_id(
                        conn,
                        &context.tenant_id,
                        &subscription_id,
                    )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let mrr = subscription.mrr_cents;

                    let event = diesel_models::subscription_events::SubscriptionEvent {
                        id: Uuid::now_v7(),
                        subscription_id,
                        event_type: SubscriptionEventType::Cancelled.into(),
                        details: None, // TODO reason etc
                        created_at: chrono::Utc::now().naive_utc(),
                        mrr_delta: Some(-(mrr as i64)),
                        bi_mrr_movement_log_id: None,
                        applies_to: billing_end_date,
                    };

                    event
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    Ok(res)
                }
                    .scope_boxed()
            })
            .await?;

        let subscription: domain::Subscription = db_subscription.into();

        Ok(subscription)
    }

    async fn list_subscriptions(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
        plan_id: Option<Uuid>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>> {
        let mut conn = self.get_conn().await?;

        let db_subscriptions = diesel_models::subscriptions::Subscription::list_subscriptions(
            &mut conn,
            tenant_id,
            customer_id,
            plan_id,
            pagination.into(),
        )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Subscription> = PaginatedVec {
            items: db_subscriptions
                .items
                .into_iter()
                .map(|s| s.into())
                .collect(),
            total_pages: db_subscriptions.total_pages,
            total_results: db_subscriptions.total_results,
        };

        Ok(res)
    }
}

fn process_create_subscription_components(
    param: &Option<CreateSubscriptionComponents>,
    map: &HashMap<Uuid, Vec<domain::PriceComponent>>,
    sub: &SubscriptionNew,
) -> Result<Vec<SubscriptionComponentNewInternal>, StoreError> {
    if param.is_none() {
        return Ok(vec![]);
    }
    let param = param.as_ref().unwrap();

    let &CreateSubscriptionComponents {
        parameterized_components,
        overridden_components,
        extra_components,
        remove_components,
    } = &param;

    let binding = vec![];
    let plan_price_components = map.get(&sub.plan_version_id).unwrap_or(&binding);

    let mut processed_components = Vec::new();
    let mut removed_components = Vec::new();


    // TODO should we add a quick_param or something to not require the component id when creating subscription without complex parameterization ?
    // basically a top level params with period, initial slots, committed capacity, that can be overriden at the component level

    let all_ids = parameterized_components
        .iter()
        .map(|p| p.component_id)
        .chain(overridden_components.iter().map(|o| o.component_id))
        .chain(remove_components.iter().cloned())
        .sorted()
        .collect::<Vec<_>>();

    let plan_price_components_ids = plan_price_components
        .iter()
        .map(|c| c.id)
        .sorted()
        .collect::<Vec<_>>();

    if all_ids != plan_price_components_ids {
        return Err(StoreError::InvalidPriceComponents(
            "Ids provided do not match plan price components".to_string(),
        ));
    }

    for c in plan_price_components {
        let c = c.clone();
        let component_id = c.id;

        // Check parameterized_components
        if let Some(parameterized) = parameterized_components
            .iter()
            .find(|p| p.component_id == component_id)
        {
            processed_components.push(apply_parameterization(&c, &parameterized.parameters)?);
            continue;
        }

        // Check overridden_components
        if let Some(overridden) = overridden_components
            .iter()
            .find(|o| o.component_id == component_id)
        {
            let mut component = overridden.component.clone();
            component.is_override = true;
            processed_components.push(component);
            continue;
        }

        // Check if the component is in remove_components
        if remove_components.contains(&component_id) {
            removed_components.push(component_id);
            continue;
        }

        let (period, fee) = match c.fee {
            FeeType::Rate { rates } => {
                if rates.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected a single rate or a parametrized component, found: {}",
                        rates.len()
                    )));
                }
                (
                    rates[0].term.as_subscription_billing_period(),
                    SubscriptionFee::Rate {
                        rate: rates[0].price,
                    },
                )
            }
            FeeType::Slot {
                minimum_count,
                quota,
                slot_unit_name,
                rates,
                ..
            } => {
                if rates.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected a single rate or a parametrized component, found: {}",
                        rates.len()
                    )));
                }

                (
                    rates[0].term.as_subscription_billing_period(),
                    SubscriptionFee::Slot {
                        unit: slot_unit_name.clone(),
                        unit_rate: rates[0].price,
                        min_slots: minimum_count,
                        max_slots: quota,
                        initial_slots: minimum_count.unwrap_or(0),
                    },
                )
            }
            FeeType::Capacity {
                metric_id,
                thresholds,
            } => {
                if thresholds.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected either a single threshold or a parametrized component, found: {}",
                        thresholds.len()
                    )));
                }

                (
                    domain::enums::SubscriptionFeeBillingPeriod::Monthly,
                    SubscriptionFee::Capacity {
                        metric_id: metric_id.clone(),
                        overage_rate: thresholds[0].per_unit_overage,
                        included: thresholds[0].included_amount,
                        rate: thresholds[0].price,
                    },
                )
            }

            FeeType::OneTime {
                quantity,
                unit_price,
            } => (
                domain::enums::SubscriptionFeeBillingPeriod::OneTime,
                SubscriptionFee::OneTime {
                    rate: unit_price,
                    quantity: quantity,
                },
            ),
            FeeType::Usage { metric_id, pricing } => (
                domain::enums::SubscriptionFeeBillingPeriod::Monthly,
                SubscriptionFee::Usage {
                    metric_id: metric_id,
                    model: pricing,
                },
            ),
            FeeType::ExtraRecurring {
                cadence,
                unit_price,
                quantity,
                billing_type,
            } => (
                cadence.as_subscription_billing_period(),
                SubscriptionFee::Recurring {
                    rate: unit_price,
                    quantity,
                    billing_type,
                },
            ),
        };

        // If the component is not in any of the lists, add it as is
        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: Some(c.id),
            product_item_id: c.product_item_id.clone(),
            name: c.name.clone(),
            period: period,
            fee: fee,
            is_override: false,
        });
    }

    // Add extra components
    for extra in extra_components {
        processed_components.push(extra.component.clone());
    }

    Ok(processed_components)
}

fn apply_parameterization(
    component: &domain::PriceComponent,
    parameters: &domain::ComponentParameters,
) -> Result<domain::SubscriptionComponentNewInternal, StoreError> {
    match &component.fee {
        FeeType::Rate { rates } => {
            if parameters.initial_slot_count.is_some() || parameters.committed_capacity.is_some() {
                return Err(StoreError::InvalidArgument(
                    "Unexpected parameters for rate fee".to_string(),
                ));
            }

            if let Some(billing_period) = &parameters.billing_period {
                let rate = rates
                    .iter()
                    .find(|r| &r.term == billing_period)
                    .ok_or_else(|| {
                        StoreError::InvalidArgument(format!(
                            "Rate not found for billing period: {:?}",
                            billing_period
                        ))
                    })?;
                Ok(domain::SubscriptionComponentNewInternal {
                    price_component_id: Some(component.id),
                    product_item_id: component.product_item_id.clone(),
                    name: component.name.clone(),
                    period: billing_period.as_subscription_billing_period(),
                    fee: SubscriptionFee::Rate { rate: rate.price },
                    is_override: false,
                })
            } else {
                if rates.len() != 1 {
                    return Err(StoreError::InvalidArgument(format!(
                        "Expected a single rate, found: {}",
                        rates.len()
                    )));
                }

                let rate = &rates[0];
                Ok(domain::SubscriptionComponentNewInternal {
                    price_component_id: Some(component.id),
                    product_item_id: component.product_item_id.clone(),
                    name: component.name.clone(),
                    period: rate.term.as_subscription_billing_period(),
                    fee: SubscriptionFee::Rate { rate: rate.price },
                    is_override: false,
                })
            }
        }
        FeeType::Slot {
            rates,
            minimum_count,
            slot_unit_name,
            quota,
            ..
        } => {
            let billing_period = parameters
                .billing_period
                .as_ref()
                .ok_or_else(|| StoreError::InvalidArgument("Missing billing period".to_string()))?;

            let rate = rates
                .iter()
                .find(|r| &r.term == billing_period)
                .ok_or_else(|| {
                    StoreError::InvalidArgument(format!(
                        "Rate not found for billing period: {:?}",
                        billing_period
                    ))
                })?;
            let initial_slots = parameters
                .initial_slot_count
                .unwrap_or_else(|| minimum_count.unwrap_or(0));

            if parameters.committed_capacity.is_some() {
                return Err(StoreError::InvalidArgument(
                    "Unexpected committed capacity for slot fee".to_string(),
                ));
            }
            Ok(domain::SubscriptionComponentNewInternal {
                price_component_id: Some(component.id),
                product_item_id: component.product_item_id.clone(),
                name: component.name.clone(),
                period: billing_period.as_subscription_billing_period(),
                fee: SubscriptionFee::Slot {
                    unit: slot_unit_name.clone(),
                    unit_rate: rate.price,
                    min_slots: minimum_count.clone(),
                    max_slots: quota.clone(),
                    initial_slots,
                },
                is_override: false,
            })
        }
        FeeType::Capacity {
            metric_id,
            thresholds,
        } => {
            let committed_capacity = parameters.committed_capacity.ok_or_else(|| {
                StoreError::InvalidArgument("Missing committed capacity".to_string())
            })?;

            let threshold = thresholds
                .iter()
                .find(|t| t.included_amount == committed_capacity)
                .ok_or_else(|| {
                    StoreError::InvalidArgument(format!(
                        "Threshold not found for committed capacity: {}",
                        committed_capacity
                    ))
                })?;

            if parameters.billing_period.is_some() || parameters.initial_slot_count.is_some() {
                return Err(StoreError::InvalidArgument(
                    "Unexpected parameters for capacity fee".to_string(),
                ));
            }

            Ok(domain::SubscriptionComponentNewInternal {
                price_component_id: Some(component.id),
                product_item_id: component.product_item_id.clone(),
                name: component.name.clone(),
                period: domain::enums::SubscriptionFeeBillingPeriod::Monthly, // Default to monthly, until we support period parametrization for capacity
                fee: SubscriptionFee::Capacity {
                    metric_id: metric_id.clone(),
                    overage_rate: threshold.per_unit_overage,
                    included: threshold.included_amount,
                    rate: threshold.price,
                },
                is_override: false,
            })
        }
        // all other case should fail, as they just cannot be parametrized
        FeeType::Usage { .. } | FeeType::ExtraRecurring { .. } | FeeType::OneTime { .. } => {
            Err(StoreError::InvalidArgument(format!(
                "Cannot parameterize fee type: {:?}",
                component.fee
            )))
        }
    }
}

impl SubscriptionDetails {
    fn calculate_cancellable_end_of_period_date(&self, now: NaiveDate) -> Option<NaiveDate> {
        // to calculate billing period :
        // if there is a commitment, use that commitment (currently no commitment so let's ignore)
        // else, we take the longest period from the main components (rate/slots/capacity), as that's what the user has already paid
        // else, that mean we're arrear and it's monthly.

        let standard_components = self
            .price_components
            .iter()
            .filter(|c| c.is_standard())
            .collect::<Vec<_>>();
        let period = standard_components
            .iter()
            .map(|c| c.period.clone())
            .max_by(|a, b| a.as_months().cmp(&b.as_months()))
            .and_then(|p| p.as_billing_period_opt())
            .unwrap_or(BillingPeriodEnum::Monthly);

        let periods = crate::utils::periods::calculate_periods_for_date(
            self.billing_start_date,
            self.billing_day as u32,
            now,
            &period,
        );

        periods.advance.map(|p| p.end)
    }
}
