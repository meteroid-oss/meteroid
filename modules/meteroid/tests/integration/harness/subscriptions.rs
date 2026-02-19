//! Subscription test helpers.

use common_domain::ids::SubscriptionId;
use diesel_models::customers::CustomerRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store::domain::Customer;
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::services::payment_resolution::ResolvedPaymentMethods;

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

    /// Get subscription component rows for a subscription.
    pub async fn get_subscription_components(
        &self,
        subscription_id: SubscriptionId,
    ) -> Vec<SubscriptionComponentRow> {
        let mut conn = self.conn().await;
        SubscriptionComponentRow::list_subscription_components_by_subscription(
            &mut conn,
            &TENANT_ID,
            &subscription_id,
        )
        .await
        .expect("Failed to get subscription components")
    }

    /// Resolve payment methods for a subscription.
    /// This calls the actual payment resolution service to check what payment methods
    /// are available based on the subscription's config and customer's connections.
    pub async fn resolve_subscription_payment_methods(
        &self,
        sub: &SubscriptionRow,
    ) -> ResolvedPaymentMethods {
        // Parse payment_methods_config from the subscription's JSON field
        let payment_methods_config: Option<PaymentMethodsConfig> = sub
            .payment_methods_config
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()).expect("Invalid payment_methods_config"));

        // Get the customer
        let mut conn = self.conn().await;
        let customer_row = CustomerRow::find_by_id(&mut conn, &sub.customer_id, &TENANT_ID)
            .await
            .expect("Failed to get customer");
        let customer: Customer = customer_row
            .try_into()
            .expect("Failed to convert CustomerRow to Customer");

        // Call the service to resolve payment methods
        self.services()
            .resolve_subscription_payment_methods(
                TENANT_ID,
                payment_methods_config.as_ref(),
                &customer,
            )
            .await
            .expect("Failed to resolve payment methods")
    }
}
