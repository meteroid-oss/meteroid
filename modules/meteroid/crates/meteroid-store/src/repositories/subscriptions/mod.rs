use crate::StoreResult;
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::prices::{Price, fee_type_billing_period, resolve_subscription_fee};
use crate::domain::products::Product;
use crate::domain::subscriptions::{PaymentMethodsConfig, PendingPlanChange};
use crate::domain::{
    BillableMetric, ConnectorProviderEnum, Customer, InvoicingEntity, PaginatedVec,
    PaginationRequest, Schedule, Subscription, SubscriptionComponent, SubscriptionComponentNew,
    SubscriptionDetails, SubscriptionPatch, TrialConfig,
};
use chrono::NaiveDate;
use common_domain::ids::{
    ConnectorId, CustomerId, PlanId, PriceId, ProductId, SubscriptionId, TenantId,
};
use std::collections::HashMap;

use crate::errors::StoreError;
use crate::services::validate_charge_automatically_with_provider_ids;
use crate::store::Store;
use error_stack::{Report, bail};
use itertools::Itertools;

use crate::domain::subscription_add_ons::SubscriptionAddOn;
use diesel_models::applied_coupons::AppliedCouponDetailedRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::schedules::ScheduleRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowPatch};
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::domain::pgmq::{HubspotSyncRequestEvent, HubspotSyncSubscription, PgmqQueue};
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::pgmq::PgmqInterface;
use diesel_models::PgConn;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::plans::PlanRow;
use diesel_models::scheduled_events::ScheduledEventRowNew;
use meteroid_store_macros::with_conn_delegate;

pub mod slots;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData, ScheduledEventNew};
use diesel_models::scheduled_events::ScheduledEventRow;
pub use slots::SubscriptionSlotsInterface;

pub enum CancellationEffectiveAt {
    EndOfBillingPeriod,
    Date(NaiveDate),
}

#[with_conn_delegate]
pub trait SubscriptionInterface {
    #[delegated]
    async fn get_subscription_details(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails>;

    async fn get_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription>;

    async fn insert_subscription_components(
        &self,
        tenant_id: TenantId,
        batch: Vec<SubscriptionComponentNew>,
    ) -> StoreResult<Vec<SubscriptionComponent>>;

    async fn list_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        plan_id: Option<PlanId>,
        status: Option<Vec<crate::domain::enums::SubscriptionStatusEnum>>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>>;

    async fn patch_subscription_conn_meta(
        &self,
        subscription_id: SubscriptionId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()>;

    async fn sync_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<()>;

    async fn sync_customer_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
    ) -> StoreResult<()>;

    async fn list_subscription_by_ids_global(
        &self,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<Vec<Subscription>>;

    async fn schedule_events(
        &self,
        conn: &mut PgConn,
        events: Vec<ScheduledEventNew>,
    ) -> StoreResult<Vec<ScheduledEvent>>;

    async fn patch_subscription(
        &self,
        tenant_id: TenantId,
        patch: SubscriptionPatch,
    ) -> StoreResult<Subscription>;
}

#[async_trait::async_trait]
impl SubscriptionInterface for Store {
    async fn get_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription> {
        let mut conn = self.get_conn().await?;

        let db_subscription =
            SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        db_subscription.try_into()
    }

    /// todo optimize db calls
    async fn get_subscription_details_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails> {
        let db_subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let subscription: Subscription = db_subscription.try_into()?;

        let schedules: Vec<Schedule> =
            ScheduleRow::list_schedules_by_subscription(conn, &tenant_id, &subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<_>>>()?;

        let subscription_component_rows =
            SubscriptionComponentRow::list_subscription_components_by_subscription(
                conn,
                &tenant_id,
                &subscription.id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Partition by price_id presence for resolution
        let (rows_with_price, rows_without_price): (Vec<_>, Vec<_>) = subscription_component_rows
            .into_iter()
            .partition(|row| row.price_id.is_some());

        // Legacy components: deserialize fee JSONB
        let mut subscription_components: Vec<SubscriptionComponent> = rows_without_price
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, Report<_>>>()?;

        // New-style components: resolve from Products + Prices
        if !rows_with_price.is_empty() {
            let price_ids: Vec<PriceId> = rows_with_price
                .iter()
                .filter_map(|r| r.price_id)
                .unique()
                .collect();

            let price_rows = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let prices_by_id: HashMap<PriceId, Price> = price_rows
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Price::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;

            let product_ids: Vec<ProductId> = rows_with_price
                .iter()
                .filter_map(|r| r.product_id)
                .unique()
                .collect();

            let product_rows = ProductRow::list_by_ids(conn, &product_ids, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            let products_by_id: HashMap<ProductId, Product> = product_rows
                .into_iter()
                .map(|row| {
                    let id = row.id;
                    Product::try_from(row).map(|p| (id, p))
                })
                .collect::<Result<HashMap<_, _>, _>>()?;

            for row in rows_with_price {
                let price_id = row.price_id.ok_or_else(|| {
                    Report::new(StoreError::InvalidArgument(
                        "Subscription component missing price_id after partition".to_string(),
                    ))
                })?;

                // Try to resolve from Product.fee_structure + Price.pricing
                let resolved = (|| -> Option<(SubscriptionFeeBillingPeriod, crate::domain::SubscriptionFee)> {
                    let price = prices_by_id.get(&price_id)?;
                    let product_id = row.product_id?;
                    let product = products_by_id.get(&product_id)?;
                    let fee_structure = &product.fee_structure;
                    let fee = resolve_subscription_fee(fee_structure, &price.pricing, None).ok()?;
                    let period = fee_type_billing_period(fee_structure)
                        .unwrap_or_else(|| price.cadence.as_subscription_billing_period());
                    Some((period, fee))
                })();

                let component = if let Some((period, fee)) = resolved {
                    SubscriptionComponent {
                        id: row.id,
                        price_component_id: row.price_component_id,
                        product_id: row.product_id,
                        subscription_id: row.subscription_id,
                        name: row.name,
                        period,
                        fee,
                        price_id: row.price_id,
                    }
                } else {
                    // Fallback to fee JSONB if resolution fails
                    row.try_into()?
                };

                subscription_components.push(component);
            }
        }

        let subscription_add_ons: Vec<SubscriptionAddOn> =
            SubscriptionAddOnRow::list_by_subscription_id(conn, &tenant_id, &subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<_>>>()?;

        let mut metric_ids = subscription_components
            .iter()
            .filter_map(SubscriptionComponent::metric_id)
            .collect::<Vec<_>>();

        metric_ids.extend(
            subscription_add_ons
                .iter()
                .filter_map(|sa| sa.fee.metric_id())
                .collect::<Vec<_>>(),
        );

        metric_ids = metric_ids.into_iter().unique().collect::<Vec<_>>();

        let applied_coupons =
            AppliedCouponDetailedRow::list_by_subscription_id(conn, &subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<_>>>()?;

        let billable_metrics: Vec<BillableMetric> =
            BillableMetricRow::get_by_ids(conn, &metric_ids, &subscription.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<_>>>()?;

        // Look up the linked CheckoutSession to get the checkout URL
        let checkout_url = if subscription.pending_checkout {
            use crate::jwt_claims::{ResourceAccess, generate_portal_token};
            use diesel_models::checkout_sessions::CheckoutSessionRow;

            // Find the active checkout session for this subscription
            let session = CheckoutSessionRow::get_by_subscription(conn, tenant_id, subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if let Some(session) = session {
                let token = generate_portal_token(
                    &self.settings.jwt_secret,
                    tenant_id,
                    ResourceAccess::CheckoutSession(session.id),
                )?;
                Some(format!(
                    "{}/checkout?token={}",
                    self.settings.public_url, token
                ))
            } else {
                None
            }
        } else {
            None
        };

        let customer: Customer =
            CustomerRow::find_by_id(conn, &subscription.customer_id, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;

        let invoicing_entity: InvoicingEntity =
            InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                conn,
                customer.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into();

        // Fetch trial config from plan version
        let trial_config = {
            let plan_with_version =
                PlanRow::get_with_version(conn, subscription.plan_version_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

            if let Some(version) = plan_with_version.version {
                if let Some(duration_days) = version.trial_duration_days {
                    if duration_days > 0 {
                        // If there's a trialing_plan_id, fetch its name
                        let trialing_plan_name =
                            if let Some(trialing_plan_id) = &version.trialing_plan_id {
                                PlanRow::get_overview_by_id(conn, *trialing_plan_id, tenant_id)
                                    .await
                                    .ok()
                                    .map(|p| p.name)
                            } else {
                                None
                            };

                        Some(TrialConfig {
                            duration_days: duration_days as u32,
                            is_free: version.trial_is_free,
                            trialing_plan_id: version.trialing_plan_id,
                            trialing_plan_name,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Fetch pending plan change from scheduled events
        let pending_plan_change = {
            let pending_events = ScheduledEventRow::get_pending_events_for_subscription(
                conn,
                subscription.id,
                &tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            let plan_change_event = pending_events.into_iter().find_map(|row| {
                let event: crate::domain::scheduled_events::ScheduledEvent = row.try_into().ok()?;
                match event.event_data {
                    ScheduledEventData::ApplyPlanChange {
                        new_plan_version_id,
                        ..
                    } => Some((new_plan_version_id, event.scheduled_time)),
                    _ => None,
                }
            });

            if let Some((new_plan_version_id, scheduled_time)) = plan_change_event {
                let plan_with_version =
                    PlanRow::get_with_version(conn, new_plan_version_id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                Some(PendingPlanChange {
                    new_plan_name: plan_with_version.plan.name,
                    new_plan_version_id,
                    effective_date: scheduled_time.date(),
                })
            } else {
                None
            }
        };

        Ok(SubscriptionDetails {
            subscription,
            price_components: subscription_components,
            add_ons: subscription_add_ons,
            applied_coupons,
            metrics: billable_metrics,
            schedules,
            checkout_url,
            customer,
            invoicing_entity,
            trial_config,
            pending_plan_change,
        })
    }

    async fn insert_subscription_components(
        &self,
        _tenant_id: TenantId,
        batch: Vec<SubscriptionComponentNew>,
    ) -> StoreResult<Vec<SubscriptionComponent>> {
        let mut conn = self.get_conn().await?;

        // TODO update mrr

        let insertable_batch: Vec<SubscriptionComponentRowNew> = batch
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        SubscriptionComponentRow::insert_subscription_component_batch(
            &mut conn,
            insertable_batch.iter().collect(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(|v| {
            v.into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()
        })?
    }

    async fn list_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        plan_id: Option<PlanId>,
        status: Option<Vec<crate::domain::enums::SubscriptionStatusEnum>>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>> {
        let mut conn = self.get_conn().await?;

        let status_filter = status.map(|s| s.into_iter().map(|x| x.into()).collect());

        let db_subscriptions = SubscriptionRow::list_subscriptions(
            &mut conn,
            &tenant_id,
            customer_id,
            plan_id,
            status_filter,
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Subscription> = PaginatedVec {
            items: db_subscriptions
                .items
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: db_subscriptions.total_pages,
            total_results: db_subscriptions.total_results,
        };

        Ok(res)
    }

    async fn patch_subscription_conn_meta(
        &self,
        subscription_id: SubscriptionId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        SubscriptionRow::upsert_conn_meta(
            &mut conn,
            provider.into(),
            subscription_id,
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn sync_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<()> {
        let connector = self.get_hubspot_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Hubspot connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let db_subscriptions =
            SubscriptionRow::list_subscriptions_by_ids(&mut conn, &tenant_id, &subscription_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::HubspotSync,
            db_subscriptions
                .into_iter()
                .map(|subscription| {
                    HubspotSyncRequestEvent::Subscription(Box::new(HubspotSyncSubscription {
                        id: subscription.subscription.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn list_subscription_by_ids_global(
        &self,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<Vec<Subscription>> {
        let mut conn = self.get_conn().await?;

        SubscriptionRow::list_by_ids(&mut conn, &subscription_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn sync_customer_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let req = SubscriptionRow::list_by_customer_ids(&mut conn, tenant_id, &customer_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|subscription| {
                HubspotSyncRequestEvent::Subscription(Box::new(HubspotSyncSubscription {
                    id: subscription.subscription.id,
                    tenant_id: subscription.subscription.tenant_id,
                }))
                .try_into()
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.pgmq_send_batch(PgmqQueue::HubspotSync, req).await
    }

    async fn schedule_events(
        &self,
        conn: &mut PgConn,
        events: Vec<ScheduledEventNew>,
    ) -> StoreResult<Vec<ScheduledEvent>> {
        let insertable_batch: Vec<ScheduledEventRowNew> = events
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        ScheduledEventRowNew::insert_batch(conn, &insertable_batch)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn patch_subscription(
        &self,
        tenant_id: TenantId,
        patch: SubscriptionPatch,
    ) -> StoreResult<Subscription> {
        use crate::domain::SubscriptionStatusEnum;

        let mut conn = self.get_conn().await?;

        let existing = SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, patch.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let status: SubscriptionStatusEnum = existing.subscription.status.into();
        if matches!(
            status,
            SubscriptionStatusEnum::Cancelled
                | SubscriptionStatusEnum::Completed
                | SubscriptionStatusEnum::Superseded
        ) {
            bail!(StoreError::InvalidArgument(
                "Cannot update subscription in terminal state".to_string()
            ));
        }

        let effective_charge_automatically = patch
            .charge_automatically
            .unwrap_or(existing.subscription.charge_automatically);

        let existing_payment_methods_config: Option<PaymentMethodsConfig> = existing
            .subscription
            .payment_methods_config
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError(format!("Failed to parse payment_methods_config: {}", e), e)
            })?;

        let effective_payment_methods_config = match &patch.payment_methods_config {
            Some(new_config) => new_config.clone(),
            None => existing_payment_methods_config,
        };

        if effective_charge_automatically {
            let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                &mut conn,
                existing.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            validate_charge_automatically_with_provider_ids(
                effective_charge_automatically,
                effective_payment_methods_config.as_ref(),
                invoicing_entity.card_provider_id,
                invoicing_entity.direct_debit_provider_id,
            )?;
        }

        let row_patch = SubscriptionRowPatch {
            charge_automatically: patch.charge_automatically,
            auto_advance_invoices: patch.auto_advance_invoices,
            net_terms: patch.net_terms.map(|n| n as i32),
            invoice_memo: patch.invoice_memo,
            purchase_order: patch.purchase_order,
            payment_methods_config: patch
                .payment_methods_config
                .map(|opt| {
                    opt.map(serde_json::to_value).transpose().map_err(|e| {
                        StoreError::SerdeError(
                            "Failed to serialize payment_methods_config".to_string(),
                            e,
                        )
                    })
                })
                .transpose()?,
        };

        SubscriptionRow::patch(&mut conn, &tenant_id, patch.id, &row_patch)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, patch.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .try_into()
    }
}

// fn get_event_priority(event_type:  ScheduledEventTypeEnum) -> i32 {
//     match event_type {
//         // Highest priority - must happen before other events
//         ScheduledEventTypeEnum::CancelSubscription => 100,
//         ScheduledEventTypeEnum::SuspendForNonPayment => 90,
//
//         // Payment events - high priority
//         ScheduledEventTypeEnum::AttemptPayment => 80,
//         ScheduledEventTypeEnum::RetryPayment => 75,
//         ScheduledEventTypeEnum::FinalizeInvoice => 70,
//
//         // Plan changes - medium priority
//         ScheduledEventTypeEnum::ApplyUpgrade => 60, // equal priority => arbitration
//         ScheduledEventTypeEnum::ApplyDowngrade => 60,
//
//         // Subscription management - medium priority
//         ScheduledEventTypeEnum::PauseSubscription => 50,
//         ScheduledEventTypeEnum::ResumeSubscription => 50,
//
//         // Notifications and other low-impact events
//         ScheduledEventTypeEnum::SendPaymentReminder => 20,
//         ScheduledEventTypeEnum::ApplyLatePaymentFee => 30,
//         ScheduledEventTypeEnum::MoveToDelinquent => 40,
//
//     }
// }
