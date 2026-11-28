//! Subscriptions as API v2 reads them (ADR-0013 §10): with their external id and `updated_at`,
//! listed by keyset in SQL, the collection being unbounded.
use crate::StoreResult;
use crate::domain::{Subscription, SubscriptionStatusEnum};
use crate::errors::StoreError;
use crate::store::Store;
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, PlanId, SubscriptionId, TenantId};
use diesel_models::PgConn;
use diesel_models::query::subscriptions::subscription_activity;
use diesel_models::subscriptions::{SubscriptionForDisplayRow, SubscriptionRow};
use error_stack::Report;
use std::collections::HashMap;

/// What a page of subscriptions is narrowed to.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionPageFilter {
    pub customer_id: Option<CustomerId>,
    pub plan_id: Option<PlanId>,
    pub status: Option<Vec<SubscriptionStatusEnum>>,
    /// Only those changed after it.
    pub updated_after: Option<DateTime<Utc>>,
    /// Only those created after it.
    pub created_after: Option<DateTime<Utc>>,
    /// Only those created before it.
    pub created_before: Option<DateTime<Utc>>,
    /// Only those whose trial matches.
    pub trial: Option<crate::domain::trials::TrialListFilter>,
}

/// A subscription with the handles API v2 reads beside it.
#[derive(Debug, Clone)]
pub struct ListedSubscription {
    pub subscription: Subscription,
    pub external_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl ListedSubscription {
    /// The row with when it last changed: its own row's change, else its creation, or a later
    /// change to what is read beside it.
    fn of(
        row: SubscriptionForDisplayRow,
        activity: Option<DateTime<Utc>>,
    ) -> Result<Self, Report<StoreError>> {
        let external_id = row.subscription.external_id.clone();
        let own = row
            .subscription
            .updated_at
            .unwrap_or_else(|| row.subscription.created_at.and_utc());
        Ok(ListedSubscription {
            subscription: row.try_into()?,
            external_id,
            updated_at: activity.map_or(own, |activity| activity.max(own)),
        })
    }
}

async fn activity_of(
    conn: &mut PgConn,
    ids: &[SubscriptionId],
) -> StoreResult<HashMap<SubscriptionId, DateTime<Utc>>> {
    Ok(subscription_activity(conn, ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .collect())
}

#[async_trait::async_trait]
pub trait SubscriptionPagesInterface {
    /// The tenant's subscriptions of `filter` in id order, those after `after`, at most `limit`.
    async fn list_subscriptions_after(
        &self,
        tenant_id: TenantId,
        filter: SubscriptionPageFilter,
        after: Option<SubscriptionId>,
        limit: usize,
    ) -> StoreResult<Vec<ListedSubscription>>;

    async fn get_listed_subscription(
        &self,
        tenant_id: TenantId,
        id: SubscriptionId,
    ) -> StoreResult<ListedSubscription>;
}

#[async_trait::async_trait]
impl SubscriptionPagesInterface for Store {
    async fn list_subscriptions_after(
        &self,
        tenant_id: TenantId,
        filter: SubscriptionPageFilter,
        after: Option<SubscriptionId>,
        limit: usize,
    ) -> StoreResult<Vec<ListedSubscription>> {
        let mut conn = self.get_conn().await?;
        let rows = SubscriptionForDisplayRow::list_page(
            &mut conn,
            tenant_id,
            filter.customer_id,
            filter.plan_id,
            filter
                .status
                .map(|status| status.into_iter().map(Into::into).collect()),
            filter
                .trial
                .map(|trial| trial.row_filter(common_clock::now().date_naive())),
            filter.updated_after,
            filter.created_after.map(|at| at.naive_utc()),
            filter.created_before.map(|at| at.naive_utc()),
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        let ids: Vec<_> = rows.iter().map(|row| row.subscription.id).collect();
        let mut activity = activity_of(&mut conn, &ids).await?;
        rows.into_iter()
            .map(|row| {
                let changed = activity.remove(&row.subscription.id);
                ListedSubscription::of(row, changed)
            })
            .collect()
    }

    async fn get_listed_subscription(
        &self,
        tenant_id: TenantId,
        id: SubscriptionId,
    ) -> StoreResult<ListedSubscription> {
        let mut conn = self.get_conn().await?;
        let row = SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let changed = activity_of(&mut conn, &[id]).await?.remove(&id);
        ListedSubscription::of(row, changed)
    }
}
