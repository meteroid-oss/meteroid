//! The subscriptions credit grants belong to: their plans, which name the grants' groups, and
//! whether a new grant may belong to one.
use crate::domain::credit::CreditGrant;
use crate::domain::credit_groups::GrantPlans;
use crate::errors::StoreError;
use crate::repositories::credit::customer_credit::normalized_currency;
use crate::services::credit::{invalid, scope::owning_subscription_tx};
use crate::{Store, StoreResult};

use common_domain::ids::{CustomerId, SubscriptionId, TenantId};
use diesel_models::subscriptions::SubscriptionRow;
use error_stack::Report;
use std::collections::HashSet;

impl Store {
    /// One lookup for every subscription `grants` belong to; none when all are customer-wide.
    pub async fn credit_grant_plans<'a>(
        &self,
        tenant_id: TenantId,
        grants: impl IntoIterator<Item = &'a CreditGrant>,
    ) -> StoreResult<GrantPlans> {
        let ids: Vec<SubscriptionId> = grants
            .into_iter()
            .filter_map(|grant| grant.scope.subscription_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if ids.is_empty() {
            return Ok(GrantPlans::new());
        }
        let mut conn = self.get_conn().await?;
        Ok(
            SubscriptionRow::list_subscriptions_by_ids(&mut conn, &tenant_id, &ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|row| (row.subscription.id, row.plan_name))
                .collect(),
        )
    }

    /// Refuses a subscription a new grant cannot belong to: another customer's, one that ended,
    /// or, for money credit (`currency` set), one billed in another currency.
    pub async fn check_grant_subscription(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        subscription_id: SubscriptionId,
        currency: Option<&str>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let subscription =
            owning_subscription_tx(&mut conn, tenant_id, customer_id, subscription_id).await?;
        if subscription.ended {
            return Err(invalid(
                "the subscription has ended; credit can no longer be added to it",
            ));
        }
        if let Some(currency) = currency
            && normalized_currency(&subscription.currency) != normalized_currency(currency)
        {
            return Err(invalid(format!(
                "the subscription is billed in {}; its credit cannot be in {}",
                normalized_currency(&subscription.currency),
                normalized_currency(currency)
            )));
        }
        Ok(())
    }
}
