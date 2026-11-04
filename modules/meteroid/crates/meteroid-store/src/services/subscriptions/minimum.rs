use crate::StoreResult;
use crate::constants::Currencies;
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::{OutboxEvent, SubscriptionEvent, SubscriptionUpdateType};
use crate::domain::plan_minimums::MinimumScope;
use crate::domain::subscription_minimums::SubscriptionMinimumNew;
use crate::domain::{Subscription, SubscriptionMinimum};
use crate::errors::StoreError;
use crate::repositories::SubscriptionInterface;
use crate::services::Services;
use crate::services::subscriptions::plan_change::calculate_components_mrr_with_slots;
use crate::store::PgConn;
use chrono::Utc;
use common_domain::ids::{SubscriptionId, TenantId};
use diesel_models::enums::SubscriptionEventType;
use diesel_models::subscription_adjustments::{
    SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentRow,
};
use diesel_models::subscription_events::SubscriptionEventRow;
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

impl Services {
    /// Set or replace a subscription-level minimum commitment as an operator override.
    /// Always writes `is_override = true`, which means: a subsequent plan_change will NOT
    /// overwrite it.
    pub(in crate::services) async fn set_subscription_minimum(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        amount_subunit: i64,
        scope: MinimumScope,
    ) -> StoreResult<SubscriptionMinimum> {
        if amount_subunit <= 0 {
            return Err(Report::new(StoreError::InvalidArgument(
                "Minimum amount_subunit must be > 0".to_string(),
            )));
        }
        if let MinimumScope::Products(ids) = &scope
            && ids.is_empty()
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "Scope=Products requires at least one product id".to_string(),
            )));
        }

        let new = SubscriptionMinimumNew {
            subscription_id,
            amount_subunit,
            scope,
            is_override: true,
        };
        self.replace_subscription_minimum(tenant_id, subscription_id, Some(new))
            .await?;

        let mut conn = self.store.get_conn().await?;
        let row = SubscriptionAdjustmentRow::find_by_subscription_id(&mut conn, &subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "subscription_minimum row vanished after insert".to_string(),
                ))
            })?;
        let components = if row.includes_all_components {
            vec![]
        } else {
            SubscriptionAdjustmentComponentRow::list_by_subscription_id(&mut conn, &subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        Ok(SubscriptionMinimum::from_rows(row, components))
    }

    /// Remove the subscription-level minimum entirely. The next plan_change will re-snapshot
    /// from the plan, but until then this subscription has no floor.
    pub(in crate::services) async fn clear_subscription_minimum(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        self.replace_subscription_minimum(tenant_id, subscription_id, None)
            .await
    }

    async fn replace_subscription_minimum(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        new: Option<SubscriptionMinimumNew>,
    ) -> StoreResult<()> {
        let mut conn = self.store.get_conn().await?;
        // Tenant check.
        SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.store
            .transaction_with(&mut conn, |conn| {
                async move {
                    let lock = SubscriptionRow::lock_customer_of(conn, subscription_id).await?;
                    diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                    SubscriptionAdjustmentRow::delete_by_subscription_id(conn, &subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    if let Some(new) = new {
                        new.to_row()
                            .insert(conn)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        let comp_rows = new.component_rows();
                        if !comp_rows.is_empty() {
                            SubscriptionAdjustmentComponentRow::insert_batch(conn, &comp_rows)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                        }
                    }

                    self.restate_minimum_mrr_tx(conn, tenant_id, subscription_id)
                        .await?;

                    let subscription: Subscription =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .try_into()?;
                    let mut event: SubscriptionEvent = subscription.into();
                    event.change_type = Some(SubscriptionUpdateType::BillingConfigurationUpdated);
                    self.store
                        .internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            &Actor::System,
                            vec![OutboxEvent::subscription_updated(event)],
                        )
                        .await?;

                    Ok::<_, Report<StoreError>>(())
                }
                .scope_boxed()
            })
            .await
    }

    /// The MRR once the floor moved: booked as a movement today when the creation already was,
    /// else carried by the `Created` event still to book.
    async fn restate_minimum_mrr_tx(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<()> {
        let today = Utc::now().date_naive();
        let mut details = self
            .store
            .get_subscription_details_with_conn(conn, tenant_id, subscription_id)
            .await?;
        let precision =
            Currencies::resolve_currency_precision(&details.subscription.currency).unwrap_or(2);
        let component_lines = calculate_components_mrr_with_slots(
            conn,
            tenant_id,
            subscription_id,
            &details.price_components,
            precision,
        )
        .await?;
        let new_mrr = self
            .restate_subscription_mrr(conn, &mut details, component_lines, precision, today)
            .await?;
        let delta = new_mrr - details.subscription.mrr_cents as i64;

        let unbooked_creation = SubscriptionEventRow::fetch_by_subscription_id_and_event_type(
            conn,
            subscription_id,
            SubscriptionEventType::Created,
            details.subscription.start_date,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .filter(|event| event.bi_mrr_movement_log_id.is_none());

        match unbooked_creation {
            Some(event) if event.mrr_delta != Some(delta) => {
                SubscriptionEventRow::update_mrr_delta(conn, event.id, delta)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            }
            None if delta != 0 => {
                SubscriptionRow::update_subscription_mrr_delta(conn, subscription_id, delta)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                self.create_movement_mrr_log(conn, tenant_id, subscription_id, today, delta)
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }
}
