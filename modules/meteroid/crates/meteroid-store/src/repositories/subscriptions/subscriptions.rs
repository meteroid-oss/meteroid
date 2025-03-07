use crate::domain::enums::SubscriptionEventType;
use crate::domain::{
    BillableMetric, CreateSubscription, CreatedSubscription, CursorPaginatedVec,
    CursorPaginationRequest, PaginatedVec, PaginationRequest, Schedule, Subscription,
    SubscriptionComponent, SubscriptionComponentNew, SubscriptionDetails,
    SubscriptionInvoiceCandidate,
};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};
use chrono::NaiveDate;
use diesel_async::scoped_futures::ScopedFutureExt;
use error_stack::Report;
use itertools::Itertools;
use uuid::Uuid;

use crate::domain::subscription_add_ons::SubscriptionAddOn;
use crate::repositories::subscriptions::CancellationEffectiveAt;
use crate::repositories::SubscriptionInterface;
use common_eventbus::Event;
use diesel_models::applied_coupons::AppliedCouponDetailedRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::schedules::ScheduleRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::{
    SubscriptionComponentRow, SubscriptionComponentRowNew,
};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::jwt_claims::{PortalJwtClaims, ResourceAccess};
use common_domain::ids::{BaseId, CustomerId, PlanId, SubscriptionId, TenantId};
use error_stack::Result;
use secrecy::{ExposeSecret, SecretString};

#[async_trait::async_trait]
impl SubscriptionInterface for Store {
    async fn insert_subscription(
        &self,
        params: CreateSubscription,
        tenant_id: TenantId,
    ) -> StoreResult<CreatedSubscription> {
        self.insert_subscription_batch(vec![params], tenant_id)
            .await?
            .pop()
            .ok_or(StoreError::InsertError.into())
    }

    async fn insert_subscription_batch(
        &self,
        batch: Vec<CreateSubscription>,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<CreatedSubscription>> {
        let mut conn = self.get_conn().await?;

        // Step 1: Gather all required data
        let context = self
            .internal
            .gather_subscription_context(&mut conn, &batch, tenant_id, &self.settings.crypt_key)
            .await?;

        // Step 2 : Prepare for internal usage, compute etc
        let subscriptions = self.internal.build_subscription_details(&batch, &context)?;

        let mut results = Vec::new();
        for sub in subscriptions {
            // Step 3 : Connector stuff (create customer, create payment intent, bundle that for saving)

            let result = self
                .internal
                .setup_payment_provider(&mut conn, &sub.subscription, &sub.customer, &context)
                .await?;

            // Step 4 : Prepare for insert
            let processed = self
                .internal
                .process_subscription(&sub, &result, &context, tenant_id)?;

            results.push(processed);
        }

        // Step 5 : Insert
        let inserted = self
            .internal
            .persist_subscriptions(&mut conn, &results, tenant_id, &self.settings.jwt_secret)
            .await?;

        // Step 4: Handle post-insertion tasks
        self.internal
            .handle_post_insertion(self.eventbus.clone(), &inserted)
            .await?;

        Ok(inserted)
    }

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

        Ok(db_subscription.into())
    }

    /// todo optimize db calls
    async fn get_subscription_details(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails> {
        let mut conn = self.get_conn().await?;

        let db_subscription =
            SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let subscription: Subscription = db_subscription.into();

        let schedules: Vec<Schedule> =
            ScheduleRow::list_schedules_by_subscription(&mut conn, &tenant_id, &subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|s| s.try_into())
                .collect::<Result<Vec<_>, _>>()?;

        let subscription_components: Vec<SubscriptionComponent> =
            SubscriptionComponentRow::list_subscription_components_by_subscription(
                &mut conn,
                &tenant_id,
                &subscription.id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Result<Vec<_>, _>>()?;

        let subscription_add_ons: Vec<SubscriptionAddOn> =
            SubscriptionAddOnRow::list_by_subscription_id(&mut conn, &tenant_id, &subscription.id)
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

        let applied_coupons =
            AppliedCouponDetailedRow::list_by_subscription_id(&mut conn, &subscription.id)
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

        let checkout_token = if subscription.pending_checkout {
            let jwt =
                generate_checkout_token(&self.settings.jwt_secret, tenant_id, subscription.id)?;
            Some(jwt)
        } else {
            None
        };

        Ok(SubscriptionDetails {
            subscription,
            price_components: subscription_components,
            add_ons: subscription_add_ons,
            applied_coupons,
            metrics: billable_metrics,
            schedules,
            checkout_token,
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
                .map(|e| e.try_into())
                .collect::<Result<Vec<_>, _>>()
        })?
    }

    async fn cancel_subscription(
        &self,
        subscription_id: SubscriptionId,
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
                        subscription_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let mrr = subscription.subscription.mrr_cents;

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
                subscription.id.as_uuid(),
                subscription.tenant_id.as_uuid(),
            ))
            .await;

        Ok(subscription)
    }

    async fn list_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        plan_id: Option<PlanId>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>> {
        let mut conn = self.get_conn().await?;

        let db_subscriptions = SubscriptionRow::list_subscriptions(
            &mut conn,
            &tenant_id,
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

pub fn generate_checkout_token(
    jwt_secret: &SecretString,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
) -> StoreResult<String> {
    let claims = serde_json::to_value(PortalJwtClaims::new(
        tenant_id,
        ResourceAccess::SubscriptionCheckout(subscription_id),
    ))
    .map_err(|err| StoreError::SerdeError("failed to generate JWT token".into(), err))?;

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
    )
    .map_err(|_| StoreError::InvalidArgument("failed to generate JWT token".into()))?;
    Ok(token)
}
