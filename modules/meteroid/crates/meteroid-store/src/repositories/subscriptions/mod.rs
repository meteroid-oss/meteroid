use crate::StoreResult;
use crate::domain::{
    BillableMetric, ConnectorProviderEnum, Customer, InvoicingEntity, PaginatedVec,
    PaginationRequest, Schedule, Subscription, SubscriptionComponent, SubscriptionComponentNew,
    SubscriptionDetails,
};
use chrono::NaiveDate;
use common_domain::ids::{ConnectorId, CustomerId, PlanId, SubscriptionId, TenantId};

use crate::errors::StoreError;
use crate::store::Store;
use error_stack::{Report, bail};
use itertools::Itertools;

use crate::domain::subscription_add_ons::SubscriptionAddOn;
use diesel_models::applied_coupons::AppliedCouponDetailedRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::schedules::ScheduleRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscriptions::SubscriptionRow;
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::domain::pgmq::{HubspotSyncRequestEvent, HubspotSyncSubscription, PgmqQueue};
use crate::jwt_claims::{ResourceAccess, generate_portal_token};
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::pgmq::PgmqInterface;
use diesel_models::PgConn;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::scheduled_events::ScheduledEventRowNew;
use meteroid_store_macros::with_conn_delegate;
use secrecy::SecretString;

pub mod slots;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventNew};
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

        let subscription_components: Vec<SubscriptionComponent> =
            SubscriptionComponentRow::list_subscription_components_by_subscription(
                conn,
                &tenant_id,
                &subscription.id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, Report<_>>>()?;

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

        let checkout_url = if subscription.pending_checkout {
            let url = generate_checkout_url(
                &self.settings.jwt_secret,
                &self.settings.public_url,
                tenant_id,
                subscription.id,
            )?;
            Some(url)
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
}

pub fn generate_checkout_url(
    jwt_secret: &SecretString,
    base_url: &String,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<String> {
    let token = generate_portal_token(
        jwt_secret,
        tenant_id,
        ResourceAccess::SubscriptionCheckout(subscription_id),
    )?;

    Ok(format!("{}/checkout?token={}", base_url, token))
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
