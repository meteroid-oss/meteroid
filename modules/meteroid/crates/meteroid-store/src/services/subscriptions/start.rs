//! The one way a subscription starts: every transition to Active or TrialActive from a state that
//! had not started goes through `mark_subscription_started_tx`, so what is owed at start (the
//! credit of promo codes applied before it started) follows every activation path.
use crate::StoreResult;
use crate::errors::StoreError;
use crate::hooks::CreditControlHooks;
use crate::services::credit::coupon_grant::mint_started_subscription_coupon_credit_tx;
use crate::store::PgConn;
use chrono::{DateTime, NaiveDate, Utc};
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::SubscriptionStatusEnum;
pub(crate) use diesel_models::subscription_facts::StartSettledBy;
use diesel_models::subscriptions::{SubscriptionCycleRowPatch, SubscriptionRow};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

pub(crate) enum StartTransition {
    /// First activation: a no-op once `activated_at` is set. It settles the start, `by`.
    Activate {
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
        status: SubscriptionStatusEnum,
        by: StartSettledBy,
    },
    /// TrialExpired back to Active once paid: the payment settles the start.
    TrialExpiredToActive {
        current_period_start: NaiveDate,
        current_period_end: Option<NaiveDate>,
    },
    /// A lifecycle transition of a subscription that had not started (its start date, a trial).
    Cycle(SubscriptionCycleRowPatch),
}

/// The opening period of a subscription whose fixed term ends in it stops at its end date, as a
/// renewal's does (R-084).
pub(crate) fn cut_at_term_end(
    end_date: Option<NaiveDate>,
    current_period_end: Option<NaiveDate>,
) -> Option<NaiveDate> {
    match (end_date, current_period_end) {
        (Some(end_date), Some(end)) if end_date <= end => Some(end_date),
        _ => current_period_end,
    }
}

pub(crate) async fn mark_subscription_started_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    transition: StartTransition,
) -> StoreResult<()> {
    mark_subscription_started_at_tx(
        conn,
        controls,
        tenant_id,
        subscription_id,
        transition,
        common_clock::now(),
    )
    .await
}

/// `mark_subscription_started_tx` at `at`, the moment it starts in business time: the clock, or
/// a replay's moment.
#[allow(clippy::disallowed_methods)]
pub(crate) async fn mark_subscription_started_at_tx(
    conn: &mut PgConn,
    controls: &dyn CreditControlHooks,
    tenant_id: TenantId,
    subscription_id: SubscriptionId,
    transition: StartTransition,
    at: DateTime<Utc>,
) -> StoreResult<()> {
    let starts_trial = match &transition {
        StartTransition::Activate { status, .. } => *status == SubscriptionStatusEnum::TrialActive,
        StartTransition::TrialExpiredToActive { .. } => false,
        StartTransition::Cycle(patch) => patch.status == Some(SubscriptionStatusEnum::TrialActive),
    };
    let settled_by = match &transition {
        StartTransition::Activate { by, .. } => Some(*by),
        StartTransition::TrialExpiredToActive { .. } => Some(StartSettledBy::Payment),
        StartTransition::Cycle(_) => None,
    };
    let end_date = match &transition {
        StartTransition::Cycle(_) => None,
        _ => {
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .subscription
                .end_date
        }
    };
    match transition {
        StartTransition::Activate {
            current_period_start,
            current_period_end,
            status,
            by: _,
        } => {
            let current_period_end = cut_at_term_end(end_date, current_period_end);
            SubscriptionRow::activate_subscription(
                conn,
                &subscription_id,
                &tenant_id,
                current_period_start,
                current_period_end,
                status,
            )
            .await
        }
        StartTransition::TrialExpiredToActive {
            current_period_start,
            current_period_end,
        } => {
            let current_period_end = cut_at_term_end(end_date, current_period_end);
            SubscriptionRow::transition_trial_expired_to_active(
                conn,
                &subscription_id,
                &tenant_id,
                current_period_start,
                current_period_end,
            )
            .await
        }
        StartTransition::Cycle(patch) => {
            if patch.id != subscription_id || patch.tenant_id != tenant_id {
                return Err(Report::new(StoreError::InvalidArgument(
                    "the cycle transition is for another subscription".to_owned(),
                )));
            }
            patch.patch(conn).await
        }
    }
    .map_err(Into::<Report<StoreError>>::into)?;
    if let Some(by) = settled_by {
        crate::services::facts::settle_start_tx(
            conn,
            tenant_id,
            subscription_id,
            at.naive_utc(),
            by,
        )
        .await?;
        crate::services::trials::ending::convert_awaiting_payment_tx(
            conn,
            tenant_id,
            subscription_id,
            at.naive_utc(),
        )
        .await?;
    }
    mint_started_subscription_coupon_credit_tx(conn, controls, tenant_id, subscription_id, at)
        .await?;
    crate::services::offers::materialize_started_tx(conn, controls, tenant_id, subscription_id, at)
        .await?;
    if starts_trial {
        crate::services::customer_emails::trials::enqueue_trial_started_email_tx(
            conn,
            tenant_id,
            subscription_id,
        )
        .await?;
    }
    Ok(())
}

impl crate::services::ServicesEdge {
    /// A subscription a sandbox replay starts at `start`, as the lifecycle would have: inserted not
    /// started, then activated with its first period issued and its coupons redeemed at `start`.
    pub(crate) async fn start_replayed_subscription(
        &self,
        actor: crate::domain::entity_activity::Actor,
        subscription: crate::domain::CreateSubscription,
        tenant_id: TenantId,
        start: DateTime<Utc>,
        delivery: crate::services::InvoiceDelivery,
    ) -> StoreResult<(
        crate::domain::CreatedSubscription,
        Option<crate::domain::DetailedInvoice>,
    )> {
        if start > common_clock::now() {
            return Err(Report::new(StoreError::InvalidArgument(
                "a subscription cannot be started at a future moment".to_owned(),
            )));
        }
        if start.date_naive() != subscription.subscription.start_date {
            return Err(Report::new(StoreError::InvalidArgument(
                "a subscription is started on its start date".to_owned(),
            )));
        }
        let mut conn = self.store.get_conn().await?;
        let actor = &actor;
        let subscription = &subscription;
        let (created, invoice) = self
            .services
            .transaction_reading_usage_first(&mut conn, |services, conn| {
                let batch = vec![subscription.clone()];
                async move {
                    let context = services
                        .gather_subscription_context(
                            conn,
                            &batch,
                            tenant_id,
                            &services.store.settings.crypt_key,
                        )
                        .await?;
                    let details = services.build_subscription_details(&batch, &context)?;
                    let mut processed = Vec::with_capacity(details.len());
                    for sub in details {
                        let payment = services.setup_payment_provider(&sub, &context)?;
                        let mut one = services
                            .process_subscription(&sub, &payment, &context, tenant_id, None)?;
                        one.defer_start()?;
                        processed.push(one);
                    }
                    let created = services
                        .persist_subscriptions_at(
                            conn,
                            actor,
                            &processed,
                            tenant_id,
                            &services.store.settings.public_url,
                            start,
                        )
                        .await?
                        .pop()
                        .ok_or_else(|| {
                            Report::new(StoreError::InsertError).attach("No subscription inserted")
                        })?;
                    let invoice = services
                        .replay_subscription_at_tx(conn, tenant_id, created.id, start, delivery)
                        .await?;
                    Ok((created, invoice))
                }
                .scope_boxed()
            })
            .await?;
        self.services
            .handle_post_insertion(
                actor,
                self.store.eventbus.clone(),
                std::slice::from_ref(&created),
            )
            .await?;
        Ok((created, invoice))
    }
}
