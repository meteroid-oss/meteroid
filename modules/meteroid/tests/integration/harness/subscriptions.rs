//! Subscription test helpers.

use common_domain::ids::SubscriptionId;
use diesel_models::subscriptions::SubscriptionRow;

use crate::data::ids::TENANT_ID;

use super::TestEnv;

impl TestEnv {
    /// Get a subscription row by ID.
    pub async fn get_subscription(&self, id: SubscriptionId) -> SubscriptionRow {
        let mut conn = self.conn().await;
        SubscriptionRow::get_subscription_by_id(&mut conn, &TENANT_ID, id)
            .await
            .expect("Failed to get subscription")
            .subscription
    }
}
