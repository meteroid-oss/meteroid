//! Subscription test helpers.

use common_domain::ids::{CustomerId, SubscriptionId};
use diesel_models::customers::CustomerRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store::domain::Customer;
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::services::payment_resolution::ResolvedPaymentMethods;

use crate::data::ids::TENANT_ID;

use super::TestEnv;

impl TestEnv {
    /// Get a customer by ID.
    pub async fn get_customer(&self, id: CustomerId) -> Customer {
        let mut conn = self.conn().await;
        CustomerRow::find_by_id(&mut conn, &id, &TENANT_ID)
            .await
            .expect("Failed to get customer")
            .try_into()
            .expect("Failed to convert CustomerRow to Customer")
    }

    /// How many periods of the subscription were invoiced before its current one: the cycle it is
    /// in, counted from its first billed period (after a free trial, or across a cadence change).
    pub async fn billed_cycle(&self, id: SubscriptionId) -> usize {
        let current = self.get_subscription(id).await.current_period_start;
        self.get_invoices(id)
            .await
            .iter()
            .filter(|invoice| {
                invoice.invoice_type == meteroid_store::domain::enums::InvoiceType::Recurring
                    && invoice.status != meteroid_store::domain::enums::InvoiceStatusEnum::Void
                    && invoice.billing_period_start < current
            })
            .count()
    }

    /// Get a subscription row by ID.
    pub async fn get_subscription(&self, id: SubscriptionId) -> SubscriptionRow {
        let mut conn = self.conn().await;
        SubscriptionRow::get_subscription_by_id(&mut conn, &TENANT_ID, id)
            .await
            .expect("Failed to get subscription")
            .subscription
    }

    /// The length in days of the subscription's trial fact, if it has one.
    pub async fn trial_days(&self, id: SubscriptionId) -> Option<i32> {
        let mut conn = self.conn().await;
        let row = SubscriptionRow::get_subscription_by_id(&mut conn, &TENANT_ID, id)
            .await
            .expect("Failed to get subscription");
        row.trial_from
            .zip(row.trial_until)
            .map(|(from, until)| i32::try_from((until - from).num_days()).unwrap_or(i32::MAX))
    }

    /// The component rows of a subscription in force in its stored period: a change written for
    /// the period's end shows once the renewal applies it.
    pub async fn get_subscription_components(
        &self,
        subscription_id: SubscriptionId,
    ) -> Vec<SubscriptionComponentRow> {
        let end = self
            .get_subscription(subscription_id)
            .await
            .current_period_end;
        let today = common_clock::now().date_naive();
        let day = end
            .and_then(|end| end.pred_opt())
            .map_or(today, |last| today.min(last));
        let mut conn = self.conn().await;
        SubscriptionComponentRow::list_subscription_components_by_subscription(
            &mut conn,
            &TENANT_ID,
            &subscription_id,
            Some(day),
        )
        .await
        .expect("Failed to get subscription components")
    }

    /// Get all subscription component rows (active and closed) overlapping with a period.
    pub async fn get_all_subscription_components(
        &self,
        subscription_id: SubscriptionId,
        period_start: chrono::NaiveDate,
        period_end: chrono::NaiveDate,
    ) -> Vec<SubscriptionComponentRow> {
        let mut conn = self.conn().await;
        SubscriptionComponentRow::list_component_history_for_period(
            &mut conn,
            &subscription_id,
            period_start,
            period_end,
        )
        .await
        .expect("Failed to get all subscription components")
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
