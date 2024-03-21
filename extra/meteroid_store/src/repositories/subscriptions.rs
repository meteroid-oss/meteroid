use crate::store::Store;
use crate::{domain, StoreResult};

#[async_trait::async_trait]
pub trait SubscriptionInterface {
    async fn insert_subscription(
        &self,
        subscription: domain::SubscriptionNew,
    ) -> StoreResult<domain::Subscription>;

    async fn insert_subscription_batch(
        &self,
        batch: Vec<domain::SubscriptionNew>,
    ) -> StoreResult<Vec<domain::Subscription>>;
}

#[async_trait::async_trait]
impl SubscriptionInterface for Store {
    async fn insert_subscription(
        &self,
        subscription: domain::SubscriptionNew,
    ) -> StoreResult<domain::Subscription> {
        let mut conn = self.get_conn().await?;

        let insertable_subscription: diesel_models::subscriptions::SubscriptionNew =
            subscription.into();

        insertable_subscription
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into)
    }

    async fn insert_subscription_batch(
        &self,
        batch: Vec<domain::SubscriptionNew>,
    ) -> StoreResult<Vec<domain::Subscription>> {
        let mut conn = self.get_conn().await?;

        let insertable_batch: Vec<diesel_models::subscriptions::SubscriptionNew> =
            batch.into_iter().map(|c| c.into()).collect();

        diesel_models::subscriptions::Subscription::insert_subscription_batch(
            &mut conn,
            insertable_batch,
        )
        .await
        .map_err(Into::into)
        .map(|v| v.into_iter().map(Into::into).collect())
    }
}
