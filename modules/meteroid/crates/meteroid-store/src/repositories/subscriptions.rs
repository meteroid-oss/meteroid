use crate::domain::enums::{
    BillingPeriodEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum,
    SubscriptionEventType, SubscriptionFeeBillingPeriod,
};
use crate::domain::{
    BillableMetric, BillingConfig, CreateSubscription, CreateSubscriptionAddOns,
    CreateSubscriptionComponents, CreateSubscriptionCoupons, CreatedSubscription,
    CursorPaginatedVec, CursorPaginationRequest, Customer, InlineCustomer, InlineInvoicingEntity,
    InvoicingEntity, PaginatedVec, PaginationRequest, PriceComponent, Schedule, Subscription,
    SubscriptionAddOnCustomization, SubscriptionAddOnNew, SubscriptionAddOnNewInternal,
    SubscriptionComponent, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionDetails, SubscriptionFee, SubscriptionInvoiceCandidate, SubscriptionNew,
};
use crate::errors::StoreError;
use crate::store::{PgConn, Store};
use crate::utils::decimals::ToSubunit;
use crate::{domain, StoreResult};
use chrono::{NaiveDate, NaiveTime};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::AsyncConnection;
use diesel_models::errors::{DatabaseError, DatabaseErrorContainer};
use error_stack::{report, Report};
use itertools::Itertools;
use std::collections::HashMap;
use uuid::Uuid;

use crate::constants::Currencies;
use crate::domain::add_ons::AddOn;
use crate::domain::coupons::{Coupon, CouponDiscount};
use crate::domain::subscription_add_ons::SubscriptionAddOn;
use crate::repositories::historical_rates::HistoricalRatesInterface;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::repositories::{CustomersInterface, InvoiceInterface};
use crate::utils::local_id::{IdType, LocalId};
use common_eventbus::Event;
use diesel_models::add_ons::AddOnRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::coupons::CouponRow;
use diesel_models::price_components::PriceComponentRow;
use diesel_models::query::plans::get_plan_names_by_version_ids;
use diesel_models::schedules::ScheduleRow;
use diesel_models::slot_transactions::SlotTransactionRow;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_coupons::{SubscriptionCouponRow, SubscriptionCouponRowNew};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use diesel_models::DbResult;
use rust_decimal::prelude::*;

pub enum CancellationEffectiveAt {
    EndOfBillingPeriod,
    Date(NaiveDate),
}

#[async_trait::async_trait]
pub trait SubscriptionInterface {
    async fn insert_subscription(
        &self,
        subscription: CreateSubscription,
        tenant_id: Uuid,
    ) -> StoreResult<CreatedSubscription>;

    async fn insert_subscription_batch(
        &self,
        batch: Vec<CreateSubscription>,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<CreatedSubscription>>;

    async fn get_subscription_details(
        &self,
        tenant_id: Uuid,
        subscription_id: Uuid,
    ) -> StoreResult<SubscriptionDetails>;

    async fn insert_subscription_components(
        &self,
        tenant_id: Uuid,
        batch: Vec<SubscriptionComponentNew>,
    ) -> StoreResult<Vec<SubscriptionComponent>>;

    async fn cancel_subscription(
        &self,
        subscription_id: Uuid,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        context: domain::TenantContext,
    ) -> StoreResult<Subscription>;

    async fn list_subscriptions(
        &self,
        tenant_id: Uuid,
        customer_id: Option<Uuid>,
        plan_id: Option<Uuid>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>>;

    async fn list_subscription_invoice_candidates(
        &self,
        date: NaiveDate,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<SubscriptionInvoiceCandidate>>;
}

// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant

fn calculate_mrr(
    fee: &SubscriptionFee,
    period: &SubscriptionFeeBillingPeriod,
    precision: u8,
) -> i64 {
    let mut total_cents = 0;

    let period_as_months = period.as_months() as i64;

    match fee {
        SubscriptionFee::Rate { rate } => {
            total_cents = rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Recurring { quantity, rate, .. } => {
            let total = rate * Decimal::from(*quantity);
            total_cents = total.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Capacity { rate, .. } => {
            total_cents = rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::Slot {
            initial_slots,
            unit_rate,
            ..
        } => {
            total_cents =
                (*initial_slots as i64) * unit_rate.to_subunit_opt(precision).unwrap_or(0);
        }
        SubscriptionFee::OneTime { .. } | SubscriptionFee::Usage { .. } => {
            // doesn't count as mrr
        }
    }

    let _mrr = total_cents / period_as_months;

    let mrr_monthly = Decimal::from(total_cents) / Decimal::from(period_as_months);

    mrr_monthly.to_i64().unwrap_or(0)
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

        SlotTransactionRow::fetch_by_subscription_id_and_price_component_id(
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
        params: CreateSubscription,
        tenant_id: Uuid,
    ) -> StoreResult<CreatedSubscription> {
        self.insert_subscription_batch(vec![params], tenant_id)
            .await?
            .pop()
            .ok_or(StoreError::InsertError.into())
    }

    async fn insert_subscription_batch(
        &self,
        batch: Vec<CreateSubscription>,
        tenant_id: Uuid,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let mut conn: PgConn = self.get_conn().await?;

        struct DieselModelWrapper {
            subscription: SubscriptionRowNew,
            price_components: Vec<SubscriptionComponentRowNew>,
            add_ons: Vec<SubscriptionAddOnRowNew>,
            coupons: Vec<SubscriptionCouponRowNew>,
            event: SubscriptionEventRow,
        }

        let plan_version_ids = batch
            .iter()
            .map(|c| c.subscription.plan_version_id)
            .collect::<Vec<_>>();

        let plan_names = get_plan_names_by_version_ids(&mut conn, plan_version_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let db_price_components_by_plan_version = PriceComponentRow::get_by_plan_ids(
            &mut conn,
            &batch
                .iter()
                .map(|c| c.subscription.plan_version_id)
                .collect::<Vec<_>>(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let all_add_ons: Vec<AddOn> = AddOnRow::list_by_ids(
            &mut conn,
            &batch
                .iter()
                .filter_map(|x| x.add_ons.as_ref())
                .flat_map(|x| &x.add_ons)
                .map(|x| x.add_on_id)
                .unique()
                .collect::<Vec<_>>(),
            &tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(|x| x.into_iter().map(TryInto::try_into).collect())?;

        let all_coupons: Vec<Coupon> = CouponRow::list_by_ids(
            &mut conn,
            &batch
                .iter()
                .filter_map(|x| x.coupons.as_ref())
                .flat_map(|x| &x.coupons)
                .map(|x| x.coupon_id)
                .unique()
                .collect::<Vec<_>>(),
            &tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .and_then(|x| x.into_iter().map(TryInto::try_into).collect())?;

        let customer_ids = batch
            .iter()
            .map(|c| c.subscription.customer_id)
            .collect::<Vec<_>>();

        let customers = self
            .list_customers_by_ids(customer_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let invoicing_entities = self.list_invoicing_entities(tenant_id).await?;

        // map the price components thanks to .try_into
        let price_components_by_plan_version: HashMap<Uuid, Vec<PriceComponent>> =
            db_price_components_by_plan_version
                .into_iter()
                .map(|(k, v)| {
                    let converted_vec: Result<Vec<PriceComponent>, _> =
                        v.into_iter().map(|c| c.try_into()).collect();
                    converted_vec.map(|vec| (k, vec))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;

        let mut insertable: Vec<DieselModelWrapper> = Vec::with_capacity(batch.len());

        for params in batch {
            let CreateSubscription {
                subscription,
                price_components,
                add_ons,
                coupons,
            } = params;

            // first we need to process the CreateSubscriptionComponents into Vec<SubscriptionComponentNew>
            // we will need the plan version components

            let customer = customers
                .iter()
                .find(|c| c.id == subscription.customer_id)
                .ok_or(StoreError::InsertError)?;

            let subscription_currency = &subscription.currency.clone();

            let precision = Currencies::resolve_currency_precision(subscription_currency)
                .ok_or(StoreError::InsertError)?;

            let insertable_subscription_components = process_create_subscription_components(
                &price_components,
                &price_components_by_plan_version,
                &subscription,
            )?;

            let insertable_subscription_add_ons =
                process_create_subscription_add_ons(&add_ons, &all_add_ons)?;

            // at this point we can know the period
            let period = extract_billing_period(
                &insertable_subscription_components,
                &insertable_subscription_add_ons,
            );

            let should_activate = customer.billing_config != BillingConfig::Manual;
            let insertable_subscription: SubscriptionRowNew =
                subscription.map_to_row(period, should_activate, tenant_id);

            let insertable_subscription_coupons = process_create_subscription_coupons(
                insertable_subscription.id,
                &coupons,
                &all_coupons,
            )?;

            let cmrr = insertable_subscription_components
                .iter()
                .map(|c| calculate_mrr(&c.fee, &c.period, precision))
                .sum::<i64>();

            let ao_mrr = insertable_subscription_add_ons
                .iter()
                .map(|c| calculate_mrr(&c.fee, &c.period, precision))
                .sum::<i64>();

            let mrr_delta = cmrr + ao_mrr;

            let mrr_delta = apply_coupons(
                self,
                &all_coupons,
                subscription_currency,
                Decimal::from_i64(mrr_delta).unwrap_or(Decimal::ZERO),
            )
            .await?
            .to_i64()
            .unwrap_or(0);

            let insertable_subscription_components = insertable_subscription_components
                .into_iter()
                .map(|c| SubscriptionComponentNew {
                    subscription_id: insertable_subscription.id,
                    internal: c,
                })
                .collect::<Vec<_>>();

            let insertable_subscription_add_ons = insertable_subscription_add_ons
                .into_iter()
                .map(|internal| SubscriptionAddOnNew {
                    subscription_id: insertable_subscription.id,
                    internal,
                })
                .collect::<Vec<_>>();

            let insertable_event = SubscriptionEventRow {
                id: Uuid::now_v7(),
                subscription_id: insertable_subscription.id,
                event_type: SubscriptionEventType::Created.into(),
                details: None,
                created_at: chrono::Utc::now().naive_utc(),
                mrr_delta: Some(mrr_delta),
                bi_mrr_movement_log_id: None,
                applies_to: insertable_subscription.billing_start_date,
            };

            insertable.push(DieselModelWrapper {
                subscription: insertable_subscription,
                price_components: insertable_subscription_components
                    .into_iter()
                    .map(|c| c.try_into())
                    .collect::<Result<Vec<_>, _>>()?,
                add_ons: insertable_subscription_add_ons
                    .into_iter()
                    .map(|c| c.try_into())
                    .collect::<Result<Vec<_>, _>>()?,
                coupons: insertable_subscription_coupons,
                event: insertable_event,
            })
        }

        let insertable_subscription_components = insertable
            .iter()
            .flat_map(|c| c.price_components.iter())
            .collect::<Vec<_>>();

        let insertable_subscription_add_ons = insertable
            .iter()
            .flat_map(|c| c.add_ons.iter())
            .collect::<Vec<_>>();

        let insertable_subscription_coupons = insertable
            .iter()
            .flat_map(|c| c.coupons.iter())
            .collect::<Vec<_>>();

        let insertable_subscriptions = insertable.iter().map(|c| &c.subscription).collect();

        let insertable_subscription_events: Vec<&SubscriptionEventRow> =
            insertable.iter().map(|c| &c.event).collect();

        let inserted_subscriptions = conn
            .transaction(|conn| {
                async move {
                    validate_coupons(conn, &insertable_subscription_coupons, tenant_id).await?;

                    let inserted_subscriptions: Vec<CreatedSubscription> =
                        SubscriptionRow::insert_subscription_batch(conn, insertable_subscriptions)
                            .await
                            .map_err(Into::<DatabaseErrorContainer>::into)
                            .map(|v| v.into_iter().map(Into::into).collect())?;

                    SubscriptionComponentRow::insert_subscription_component_batch(
                        conn,
                        insertable_subscription_components,
                    )
                    .await
                    .map_err(Into::<DatabaseErrorContainer>::into)?;

                    SubscriptionAddOnRow::insert_batch(conn, insertable_subscription_add_ons)
                        .await
                        .map_err(Into::<DatabaseErrorContainer>::into)?;

                    SubscriptionCouponRow::insert_batch(conn, insertable_subscription_coupons)
                        .await
                        .map_err(Into::<DatabaseErrorContainer>::into)?;

                    SubscriptionEventRow::insert_batch(conn, insertable_subscription_events)
                        .await
                        .map_err(Into::<DatabaseErrorContainer>::into)?;

                    Ok::<_, DatabaseErrorContainer>(inserted_subscriptions)
                }
                .scope_boxed()
            })
            .await?;

        // we now want to insert the invoices, ONLY for manual providers
        let insertable_invoices = inserted_subscriptions
            .iter()
            .filter(|s| s.activated_at.is_some())
            .map(|s| {
                let customer = customers
                    .iter()
                    .find(|c| c.id == s.customer_id)
                    .ok_or(StoreError::InsertError)?;

                let invoicing_entity = invoicing_entities
                    .iter()
                    .find(|c| c.id == customer.invoicing_entity_id)
                    .ok_or(StoreError::InsertError)?;

                match customer.billing_config {
                    BillingConfig::Stripe(_) => Ok(None),
                    BillingConfig::Manual => {
                        let plan_name = plan_names
                            .get(&s.plan_version_id)
                            .ok_or(StoreError::InsertError)?;

                        let sub = SubscriptionInvoiceCandidate {
                            id: s.id,
                            tenant_id: s.tenant_id,
                            customer_id: s.customer_id,
                            plan_version_id: s.plan_version_id,
                            billing_start_date: s.billing_start_date,
                            billing_end_date: s.billing_end_date,
                            billing_day: s.billing_day,
                            activated_at: s.activated_at,
                            canceled_at: s.canceled_at,
                            currency: s.currency.clone(),
                            net_terms: s.net_terms,
                            plan_name: plan_name.clone(),
                            period: s.period.clone(),
                        };

                        subscription_to_draft(&sub, customer, invoicing_entity).map(Some)
                    }
                }
            })
            .collect::<Result<Vec<Option<_>>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        // not in transaction, make sure the draft worker can pick them up
        self.insert_invoice_batch(insertable_invoices).await?;

        let _ = futures::future::join_all(inserted_subscriptions.clone().into_iter().map(|res| {
            self.eventbus.publish(Event::subscription_created(
                res.created_by,
                res.id,
                res.tenant_id,
            ))
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>();

        Ok(inserted_subscriptions)
    }

    /// todo parallelize db calls
    async fn get_subscription_details(
        &self,
        tenant_id: Uuid,
        subscription_id: Uuid,
    ) -> StoreResult<SubscriptionDetails> {
        let mut conn = self.get_conn().await?;

        let db_subscription =
            SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, &subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let subscription: Subscription = db_subscription.into();

        let schedules: Vec<Schedule> =
            ScheduleRow::list_schedules_by_subscription(&mut conn, &tenant_id, &subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?;

        let subscription_components: Vec<SubscriptionComponent> =
            SubscriptionComponentRow::list_subscription_components_by_subscription(
                &mut conn,
                &tenant_id,
                &subscription_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()?;

        let subscription_add_ons: Vec<SubscriptionAddOn> =
            SubscriptionAddOnRow::list_by_subscription_id(&mut conn, &tenant_id, &subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?;

        let mut metric_ids = subscription_components
            .iter()
            .filter_map(|sc| sc.metric_id())
            .collect::<Vec<_>>();

        metric_ids.extend(
            subscription_add_ons
                .iter()
                .filter_map(|sa| sa.fee.metric_id())
                .collect::<Vec<_>>(),
        );

        metric_ids = metric_ids.into_iter().unique().collect::<Vec<_>>();

        let subscription_coupons =
            CouponRow::list_by_subscription_id(&mut conn, &tenant_id, &subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?;

        let billable_metrics: Vec<BillableMetric> =
            BillableMetricRow::get_by_ids(&mut conn, &metric_ids, &subscription.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|m| m.try_into())
                .collect::<Result<Vec<_>, _>>()?;

        Ok(SubscriptionDetails {
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
            add_ons: subscription_add_ons,
            coupons: subscription_coupons,
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
            period: subscription.period,
        })
    }

    async fn insert_subscription_components(
        &self,
        _tenant_id: Uuid,
        batch: Vec<SubscriptionComponentNew>,
    ) -> StoreResult<Vec<SubscriptionComponent>> {
        let mut conn = self.get_conn().await?;

        // TODO update mrr

        let insertable_batch: Vec<SubscriptionComponentRowNew> = batch
            .into_iter()
            .map(|c| c.try_into())
            .collect::<Result<Vec<_>, _>>()?;

        SubscriptionComponentRow::insert_subscription_component_batch(
            &mut conn,
            insertable_batch.iter().collect(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(|v| {
            v.into_iter()
                .map(|e| e.try_into().map_err(Report::from))
                .collect::<Result<Vec<_>, _>>()
        })?
    }

    async fn cancel_subscription(
        &self,
        subscription_id: Uuid,
        reason: Option<String>,
        effective_at: CancellationEffectiveAt,
        context: domain::TenantContext,
    ) -> StoreResult<Subscription> {
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
                        CancellationEffectiveAt::Date(date) => date,
                    };

                    SubscriptionRow::cancel_subscription(
                        conn,
                        diesel_models::subscriptions::CancelSubscriptionParams {
                            subscription_id,
                            tenant_id: context.tenant_id,
                            billing_end_date,
                            canceled_at: now,
                            reason,
                        },
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let res = SubscriptionRow::get_subscription_by_id(
                        conn,
                        &context.tenant_id,
                        &subscription_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let mrr = subscription.mrr_cents;

                    let event = SubscriptionEventRow {
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

        let subscription: Subscription = db_subscription.into();

        let _ = self
            .eventbus
            .publish(Event::subscription_canceled(
                context.actor,
                subscription.id,
                subscription.tenant_id,
            ))
            .await;

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

        let db_subscriptions = SubscriptionRow::list_subscriptions(
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

    async fn list_subscription_invoice_candidates(
        &self,
        date: NaiveDate,
        pagination: CursorPaginationRequest,
    ) -> StoreResult<CursorPaginatedVec<SubscriptionInvoiceCandidate>> {
        let mut conn = self.get_conn().await?;

        let db_subscriptions = SubscriptionRow::list_subscription_to_invoice_candidates(
            &mut conn,
            date,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: CursorPaginatedVec<SubscriptionInvoiceCandidate> = CursorPaginatedVec {
            items: db_subscriptions
                .items
                .into_iter()
                .map(|s| s.into())
                .collect(),
            next_cursor: db_subscriptions.next_cursor,
        };

        Ok(res)
    }
}

fn process_create_subscription_add_ons(
    create: &Option<CreateSubscriptionAddOns>,
    add_ons: &[AddOn],
) -> Result<Vec<SubscriptionAddOnNewInternal>, StoreError> {
    let mut processed_add_ons = Vec::new();

    if let Some(create) = create {
        for cs_ao in &create.add_ons {
            let add_on = add_ons.iter().find(|x| x.id == cs_ao.add_on_id).ok_or(
                StoreError::ValueNotFound(format!("add-on {} not found", cs_ao.add_on_id)),
            )?;

            match &cs_ao.customization {
                SubscriptionAddOnCustomization::None => {
                    let (period, fee) = add_on.fee.to_subscription_fee()?;
                    processed_add_ons.push(SubscriptionAddOnNewInternal {
                        add_on_id: add_on.id,
                        name: add_on.name.clone(),
                        period,
                        fee,
                    });
                }
                SubscriptionAddOnCustomization::Override(override_) => {
                    processed_add_ons.push(SubscriptionAddOnNewInternal {
                        add_on_id: add_on.id,
                        name: override_.name.clone(),
                        period: override_.period.clone(),
                        fee: override_.fee.clone(),
                    });
                }
                SubscriptionAddOnCustomization::Parameterization(param) => {
                    let (period, fee) = add_on.fee.to_subscription_fee_parameterized(
                        &param.initial_slot_count,
                        &param.billing_period,
                        &param.committed_capacity,
                    )?;
                    processed_add_ons.push(SubscriptionAddOnNewInternal {
                        add_on_id: add_on.id,
                        name: add_on.name.clone(),
                        period,
                        fee,
                    });
                }
            }
        }
    }

    Ok(processed_add_ons)
}

fn process_create_subscription_coupons(
    subscription_id: Uuid,
    create: &Option<CreateSubscriptionCoupons>,
    coupons: &[Coupon],
) -> Result<Vec<SubscriptionCouponRowNew>, StoreError> {
    let mut processed_coupons = Vec::new();
    if let Some(create) = create {
        for cs_coupon in &create.coupons {
            let coupon = coupons.iter().find(|x| x.id == cs_coupon.coupon_id).ok_or(
                StoreError::ValueNotFound(format!("coupon {} not found", cs_coupon.coupon_id)),
            )?;

            processed_coupons.push(SubscriptionCouponRowNew {
                id: Uuid::now_v7(),
                subscription_id,
                coupon_id: coupon.id,
            });
        }
    }

    processed_coupons = processed_coupons
        .into_iter()
        .unique_by(|x| x.coupon_id)
        .collect();

    Ok(processed_coupons)
}

/// validate coupons can be applied to subscriptions
/// must be inside tx to handle concurrent inserts
async fn validate_coupons(
    tx_conn: &mut PgConn,
    subscription_coupons: &[&SubscriptionCouponRowNew],
    tenant_id: Uuid,
) -> DbResult<()> {
    if subscription_coupons.is_empty() {
        return Ok(());
    }

    let coupons_ids = subscription_coupons
        .iter()
        .map(|x| x.coupon_id)
        .unique()
        .collect::<Vec<_>>();

    let coupons = &CouponRow::list_by_ids_for_update(tx_conn, &coupons_ids, &tenant_id).await?;

    let now = chrono::Utc::now().naive_utc();

    // expired coupons
    for coupon in coupons {
        let expired = coupon.expires_at.map(|x| x <= now).unwrap_or(false);
        if expired {
            return Err(report!(DatabaseError::ValidationError(format!(
                "coupon {} is expired",
                coupon.code
            )))
            .into());
        }
    }

    let subscriptions_by_coupon: HashMap<Uuid, usize> =
        subscription_coupons.iter().counts_by(|x| x.coupon_id);

    let db_counts = CouponRow::subscriptions_count(tx_conn, &coupons_ids).await?;

    // check if the coupon has reached its redemption limit
    for (coupon_id, subscriptions_count) in subscriptions_by_coupon {
        let applied_count = db_counts.get(&coupon_id).unwrap_or(&0);
        let coupon = coupons.iter().find(|x| x.id == coupon_id).ok_or(report!(
            DatabaseError::ValidationError(format!("coupon {} not found", coupon_id))
        ))?;

        if let Some(redemption_limit) = coupon.redemption_limit {
            if (redemption_limit as i64) < (subscriptions_count as i64 + *applied_count) {
                return Err(report!(DatabaseError::ValidationError(format!(
                    "coupon {} has reached its maximum redemptions",
                    coupon.code
                )))
                .into());
            }
        }
    }

    Ok(())
}

async fn apply_coupons(
    store: &Store,
    coupons: &[Coupon],
    subscription_currency: &String,
    amount: Decimal,
) -> StoreResult<Decimal> {
    if (amount == Decimal::ZERO) || coupons.is_empty() {
        return Ok(amount);
    }

    let mut total = amount;

    for coupon in coupons {
        if !coupon.is_infinite() {
            continue;
        }

        match &coupon.discount {
            CouponDiscount::Percentage(percentage) => {
                total = total * percentage / Decimal::new(100, 0)
            }
            CouponDiscount::Fixed {
                currency,
                amount: fixed_amount,
            } => {
                let discount_amount = if currency != subscription_currency {
                    let rate = store
                        .get_historical_rate(
                            currency,
                            subscription_currency,
                            chrono::Utc::now().date_naive(),
                        )
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .ok_or(StoreError::ValueNotFound(format!(
                            "historical rate from {} to {}",
                            currency, subscription_currency
                        )))?
                        .rate;

                    fixed_amount * Decimal::from_f32(rate).unwrap_or(Decimal::ZERO)
                } else {
                    *fixed_amount
                };

                total = (total - discount_amount).max(Decimal::ZERO)
            }
        }
    }

    Ok(total)
}

fn extract_billing_period(
    components: &[SubscriptionComponentNewInternal],
    add_ons: &[SubscriptionAddOnNewInternal],
) -> BillingPeriodEnum {
    components
        .iter()
        .map(|x| &x.period)
        .chain(add_ons.iter().map(|x| &x.period))
        .filter_map(|x| x.as_billing_period_opt())
        .min()
        .unwrap_or(BillingPeriodEnum::Monthly)
}

fn process_create_subscription_components(
    param: &Option<CreateSubscriptionComponents>,
    map: &HashMap<Uuid, Vec<PriceComponent>>,
    sub: &SubscriptionNew,
) -> Result<Vec<SubscriptionComponentNewInternal>, StoreError> {
    let mut processed_components = Vec::new();

    let (parameterized_components, overridden_components, extra_components, remove_components) =
        if let Some(p) = param {
            (
                &p.parameterized_components,
                &p.overridden_components,
                &p.extra_components,
                &p.remove_components,
            )
        } else {
            (&Vec::new(), &Vec::new(), &Vec::new(), &Vec::new())
        };

    let binding = vec![];
    let plan_price_components = map.get(&sub.plan_version_id).unwrap_or(&binding);

    let mut removed_components = Vec::new();

    // TODO should we add a quick_param or something to not require the component id when creating subscription without complex parameterization ?
    // basically a top level params with period, initial slots, committed capacity, that can be overriden at the component level

    for c in plan_price_components {
        let c = c.clone();
        let component_id = c.id;

        // Check parameterized_components
        if let Some(parameterized) = parameterized_components
            .iter()
            .find(|p| p.component_id == component_id)
        {
            let (period, fee) = c.fee.to_subscription_fee_parameterized(
                &parameterized.parameters.initial_slot_count,
                &parameterized.parameters.billing_period,
                &parameterized.parameters.committed_capacity,
            )?;
            processed_components.push(SubscriptionComponentNewInternal {
                price_component_id: Some(c.id),
                product_item_id: c.product_item_id,
                name: c.name.clone(),
                period,
                fee,
                is_override: false,
            });
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

        let (period, fee) = c.fee.to_subscription_fee()?;

        // If the component is not in any of the lists, add it as is
        processed_components.push(SubscriptionComponentNewInternal {
            price_component_id: Some(c.id),
            product_item_id: c.product_item_id,
            name: c.name.clone(),
            period,
            fee,
            is_override: false,
        });
    }

    // Add extra components
    for extra in extra_components {
        processed_components.push(extra.component.clone());
    }

    Ok(processed_components)
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

        Some(periods.advance.end)
    }
}

pub fn subscription_to_draft(
    subscription: &SubscriptionInvoiceCandidate,
    customer: &Customer,
    invoicing_entity: &InvoicingEntity,
) -> StoreResult<domain::invoices::InvoiceNew> {
    let cust_bill_cfg = &customer.billing_config;
    let billing_start_date = subscription.billing_start_date;
    let billing_day = subscription.billing_day as u32;

    let period = crate::utils::periods::calculate_period_range(
        billing_start_date,
        billing_day,
        0, // TODO ???
        &subscription.period,
    );

    let invoicing_provider = match cust_bill_cfg {
        BillingConfig::Stripe(_) => InvoicingProviderEnum::Stripe,
        BillingConfig::Manual => InvoicingProviderEnum::Manual,
    };

    let due_date = (period.end + chrono::Duration::days(subscription.net_terms as i64))
        .and_time(NaiveTime::MIN);

    // should we have a draft number ? TODO re-set optional, and also implement it in finalize, and fetch from tenant config
    let invoice_number = "draft";

    let invoice = crate::domain::invoices::InvoiceNew {
        tenant_id: subscription.tenant_id,
        customer_id: subscription.customer_id,
        subscription_id: Some(subscription.id),
        plan_version_id: Some(subscription.plan_version_id),
        invoice_type: InvoiceType::Recurring,
        currency: subscription.currency.clone(),
        external_invoice_id: None,
        invoicing_provider,
        line_items: vec![], // TODO
        issued: false,
        issue_attempts: 0,
        last_issue_attempt_at: None,
        last_issue_error: None,
        data_updated_at: None,
        status: InvoiceStatusEnum::Draft,
        external_status: None,
        invoice_date: period.end,
        finalized_at: None,
        subtotal: 0,
        subtotal_recurring: 0,
        tax_rate: 0,
        tax_amount: 0,
        total: 0,
        amount_due: 0,
        net_terms: subscription.net_terms,
        reference: None,
        memo: None,
        local_id: LocalId::generate_for(IdType::Invoice),
        due_at: Some(due_date),
        plan_name: None, // TODO
        invoice_number: invoice_number.to_string(),
        customer_details: InlineCustomer {
            id: subscription.customer_id,
            name: customer.name.clone(), // TODO
            billing_address: customer.billing_address.clone(),
            vat_number: None,
            email: customer.email.clone(),
            alias: customer.alias.clone(),
            snapshot_at: chrono::Utc::now().naive_utc(),
        },
        seller_details: InlineInvoicingEntity {
            id: invoicing_entity.id,
            legal_name: invoicing_entity.legal_name.clone(),
            vat_number: invoicing_entity.vat_number.clone(),
            address: invoicing_entity.address(),
            snapshot_at: chrono::Utc::now().naive_utc(),
        },
    };

    Ok(invoice)
}
