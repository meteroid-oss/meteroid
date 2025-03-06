use crate::domain::enums::SubscriptionEventType;
use crate::domain::{
    CreateSubscription, CreateSubscriptionAddOns, CreateSubscriptionComponents,
    CreatedSubscription, Customer, InvoicingEntityProviderSensitive, SubscriptionAddOnNew,
    SubscriptionAddOnNewInternal, SubscriptionComponentNew, SubscriptionComponentNewInternal,
    SubscriptionNew,
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::store::{PgConn, StoreInternal};
use crate::StoreResult;
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::{Report, Result};
use futures::TryFutureExt;
use secrecy::SecretString;
use std::sync::Arc;
use uuid::Uuid;

use crate::constants::{Currencies, Currency};
use crate::domain::coupons::Coupon;
use crate::repositories::subscriptions::context::SubscriptionCreationContext;
use crate::repositories::subscriptions::payment_method::PaymentSetupResult;
use crate::repositories::subscriptions::subscriptions::generate_checkout_token;
use crate::repositories::subscriptions::utils::{
    apply_coupons, calculate_mrr, extract_billing_period, process_create_subscription_add_ons,
    process_create_subscription_components, process_create_subscription_coupons,
};
use common_domain::ids::{BaseId, TenantId};
use common_eventbus::{Event, EventBus};
use diesel_models::applied_coupons::AppliedCouponRowNew;
use diesel_models::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};
use tracing::log;
// PROCESS

#[derive(Debug)]
pub struct ProcessedSubscription {
    subscription: SubscriptionRowNew,
    components: Vec<SubscriptionComponentRowNew>,
    add_ons: Vec<SubscriptionAddOnRowNew>,
    coupons: Vec<AppliedCouponRowNew>,
    event: SubscriptionEventRow,
}

pub struct DetailedSubscription {
    pub subscription: SubscriptionNew,
    components: Vec<SubscriptionComponentNewInternal>,
    add_ons: Vec<SubscriptionAddOnNewInternal>,
    coupons: Vec<Coupon>,
    pub customer: Customer,
    pub invoicing_entity: InvoicingEntityProviderSensitive,
    tenant_id: TenantId,
    currency: Currency,
}

impl StoreInternal {
    pub(super) fn build_subscription_details(
        &self,
        batch: &Vec<CreateSubscription>,
        context: &SubscriptionCreationContext,
        tenant_id: TenantId,
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
                    .ok_or(Report::new(StoreError::InsertError))?;

                let invoicing_entity = context
                    .get_invoicing_entity_providers_for_customer(customer)
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(
                            "No invoicing entity found for customer".to_string(),
                        ))
                    })?;

                let plan = context
                    .plans
                    .iter()
                    .find(|p| p.version_id == subscription.plan_version_id)
                    .ok_or(Report::new(StoreError::ValueNotFound(
                        "Plan id not found".to_string(),
                    )))?;

                let subscription_currency = &plan.currency.clone();

                let currency = Currencies::resolve_currency(subscription_currency)
                    .ok_or(StoreError::InsertError)?
                    .clone();

                let components =
                    self.process_components(price_components, subscription, context)?;
                let subscription_add_ons = self.process_add_ons(add_ons, context)?;

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
                        invoicing_entity: invoicing_entity.clone(),
                        tenant_id,
                        currency,
                    }
                })
            })
            .collect::<Result<Vec<DetailedSubscription>, _>>();

        res
    }

    pub(super) fn process_subscription(
        &self,
        sub: &DetailedSubscription,
        payment_setup_result: &PaymentSetupResult,
        // params: CreateSubscription,
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

        let subscription_row = sub.subscription.map_to_row(
            period,
            tenant_id,
            plan,
            payment_setup_result.customer_connection_id,
            payment_setup_result.payment_method,
            payment_setup_result.checkout,
        );
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

        Ok(ProcessedSubscription {
            subscription: subscription_row,
            components,
            add_ons: subscription_add_ons,
            coupons: subscription_coupons,
            event,
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
        coupons: &Vec<Coupon>,
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

// PERSIST

impl StoreInternal {
    pub(super) async fn persist_subscriptions(
        &self,
        conn: &mut PgConn,
        processed: &[ProcessedSubscription],
        tenant_id: TenantId,
        jwt_secret: &SecretString,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let res = self
            .transaction_with(conn, |conn| {
                async move {
                    // Flatten collections for batch insertion
                    let subscriptions: Vec<_> = processed.iter().map(|p| &p.subscription).collect();
                    let components: Vec<_> = processed.iter().flat_map(|p| &p.components).collect();
                    let add_ons: Vec<_> = processed.iter().flat_map(|p| &p.add_ons).collect();
                    let coupons: Vec<_> = processed.iter().flat_map(|p| &p.coupons).collect();
                    let events: Vec<_> = processed.iter().map(|p| &p.event).collect();

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

                    self.insert_created_outbox_events_tx(conn, &inserted, tenant_id)
                        .await?;

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

    // TODO if ON_START & now => activate. Also, invoice ?
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
