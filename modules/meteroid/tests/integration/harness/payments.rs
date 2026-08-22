//! Payment test helpers.
//!
//! Provides TestEnv methods for seeding and querying payment-related test data.
//! The actual seed logic lives in `data/payment.rs`.

use common_domain::ids::{
    CheckoutSessionId, ConnectorId, CustomerConnectionId, CustomerId, CustomerPaymentMethodId,
};
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::invoicing_entities::InvoicingEntityRowProvidersPatch;
use diesel_models::payments::PaymentTransactionRow;

use crate::data::ids;

use super::TestEnv;

impl TestEnv {
    // ========================================================================
    // Seed methods (thin wrappers around data::payment functions)
    // ========================================================================

    /// Seed the mock payment provider.
    pub async fn seed_mock_payment_provider(&self, fail_payment_intent: bool) {
        crate::data::payment::run_mock_payment_provider_seed(self.pool(), fail_payment_intent)
            .await;
    }

    /// Update the mock payment provider's fail_payment_intent flag.
    pub async fn set_mock_payment_failure(&self, fail: bool) {
        crate::data::payment::update_mock_payment_provider_fail(self.pool(), fail).await;
    }

    /// Set the mock provider's charge behavior: "succeeded", "pending"
    /// (async settlement), "requires_action" (3DS), or "failed".
    pub async fn set_mock_charge_behavior(&self, charge_behavior: &str) {
        crate::data::payment::set_mock_charge_behavior(self.pool(), charge_behavior).await;
    }

    /// Seed the second mock payment provider.
    pub async fn seed_mock_payment_provider_2(&self) {
        crate::data::payment::run_mock_payment_provider_2_seed(self.pool()).await;
    }

    /// Seed customer payment methods for Uber & Spotify.
    pub async fn seed_customer_payment_methods(&self) {
        crate::data::payment::run_customer_payment_methods_seed(self.pool()).await;
    }

    /// Seed customer payment methods for the secondary provider.
    pub async fn seed_customer_payment_methods_provider_2(&self) {
        crate::data::payment::run_customer_payment_methods_provider_2_seed(self.pool()).await;
    }

    /// Seed a SEPA direct debit method for Uber and make it the default, so checkout
    /// charges go down the async rail instead of the card one.
    pub async fn seed_uber_sepa_payment_method(&self) {
        let connection_id = crate::data::payment::get_or_create_customer_connection(
            self.pool(),
            ids::CUST_UBER_ID,
            ids::CUST_UBER_CONNECTION_ID,
            ids::MOCK_CONNECTOR_ID,
            "mock_cus_uber",
        )
        .await;

        crate::data::payment::create_customer_sepa_payment_method(
            self.pool(),
            ids::CUST_UBER_ID,
            connection_id,
            ids::CUST_UBER_SEPA_METHOD_ID,
            "mock_pm_uber_sepa",
        )
        .await;

        crate::data::payment::set_customer_default_payment_method(
            self.pool(),
            ids::CUST_UBER_ID,
            ids::CUST_UBER_SEPA_METHOD_ID,
        )
        .await;
    }

    /// Number of dunning retries queued for this invoice.
    pub async fn pending_payment_retries(
        &self,
        subscription_id: common_domain::ids::SubscriptionId,
        invoice_id: common_domain::ids::InvoiceId,
    ) -> usize {
        use meteroid_store::domain::scheduled_events::ScheduledEventData;

        let mut conn = self.conn().await;
        let events = diesel_models::scheduled_events::ScheduledEventRow::
            get_pending_events_for_subscription(&mut conn, subscription_id, &ids::TENANT_ID)
                .await
                .expect("failed to list pending scheduled events");

        events
            .into_iter()
            .filter(|row| {
                matches!(
                    serde_json::from_value::<ScheduledEventData>(row.event_data.clone()),
                    Ok(ScheduledEventData::RetryPayment { invoice_id: queued }) if queued == invoice_id
                )
            })
            .count()
    }

    /// Seed both payment provider and customer payment methods.
    pub async fn seed_payments(&self) {
        self.seed_mock_payment_provider(false).await;
        self.seed_customer_payment_methods().await;
    }

    /// Seed both providers and customer payment methods for both.
    pub async fn seed_dual_providers(&self) {
        self.seed_mock_payment_provider(false).await;
        self.seed_mock_payment_provider_2().await;
        self.seed_customer_payment_methods().await;
        self.seed_customer_payment_methods_provider_2().await;
    }

    /// Seed a Stancer connector as the card provider, with a pre-existing
    /// connection + payment method (see `run_stancer_provider_seed`).
    pub async fn seed_stancer_payments(&self) {
        crate::data::payment::run_stancer_provider_seed(self.pool()).await;
        crate::data::payment::run_customer_payment_methods_stancer_seed(self.pool()).await;
    }

    /// Seed direct debit provider (only DD, no card).
    pub async fn seed_direct_debit_provider(&self) {
        crate::data::payment::run_direct_debit_provider_seed(self.pool()).await;
    }

    /// Seed same provider for both card and direct debit.
    pub async fn seed_card_and_direct_debit_same_provider(&self) {
        crate::data::payment::run_card_and_dd_same_provider_seed(self.pool()).await;
    }

    /// Seed a bank account for bank transfer testing.
    pub async fn seed_bank_account(&self) {
        crate::data::payment::run_bank_account_seed(self.pool()).await;
    }

    // ========================================================================
    // Query methods
    // ========================================================================

    /// Get payment method details (connection_id, connector_id) if found.
    #[allow(dead_code)]
    pub async fn get_payment_method_provider(
        &self,
        payment_method_id: CustomerPaymentMethodId,
    ) -> Option<(CustomerConnectionId, ConnectorId)> {
        use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;

        let mut conn = self
            .pool()
            .get()
            .await
            .expect("couldn't get db connection from pool");

        let payment_method =
            CustomerPaymentMethodRow::get_by_id(&mut conn, &ids::TENANT_ID, &payment_method_id)
                .await
                .ok()?;

        let connections = CustomerConnectionRow::list_connections_by_customer_id(
            &mut conn,
            &ids::TENANT_ID,
            &payment_method.customer_id,
        )
        .await
        .expect("Failed to list connections");

        let connection = connections
            .into_iter()
            .find(|c| c.id == payment_method.connection_id)?;

        Some((connection.id, connection.connector_id))
    }

    /// List payment transactions linked to a checkout session.
    pub async fn get_transactions_by_checkout_session(
        &self,
        checkout_session_id: CheckoutSessionId,
    ) -> Vec<PaymentTransactionRow> {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::payment_transaction::dsl as pt;

        let mut conn = self
            .pool()
            .get()
            .await
            .expect("couldn't get db connection from pool");

        pt::payment_transaction
            .filter(pt::checkout_session_id.eq(checkout_session_id))
            .load(&mut conn)
            .await
            .expect("Failed to query payment transactions")
    }

    /// Switch the invoicing entity to use the secondary payment provider.
    pub async fn switch_to_provider_2(&self) {
        let mut conn = self
            .pool()
            .get()
            .await
            .expect("couldn't get db connection from pool");

        InvoicingEntityRowProvidersPatch {
            id: ids::INVOICING_ENTITY_ID,
            card_provider_id: Some(Some(ids::MOCK_CONNECTOR_2_ID)),
            direct_debit_provider_id: None,
            bank_account_id: None,
        }
        .patch_invoicing_entity_providers(&mut conn, ids::TENANT_ID)
        .await
        .expect("Failed to switch provider");
    }

    /// Switch the invoicing entity back to the primary payment provider.
    #[allow(dead_code)]
    pub async fn switch_to_provider_1(&self) {
        let mut conn = self
            .pool()
            .get()
            .await
            .expect("couldn't get db connection from pool");

        InvoicingEntityRowProvidersPatch {
            id: ids::INVOICING_ENTITY_ID,
            card_provider_id: Some(Some(ids::MOCK_CONNECTOR_ID)),
            direct_debit_provider_id: None,
            bank_account_id: None,
        }
        .patch_invoicing_entity_providers(&mut conn, ids::TENANT_ID)
        .await
        .expect("Failed to switch provider");
    }

    /// Get all customer connections for a customer.
    pub async fn get_customer_connections(
        &self,
        customer_id: CustomerId,
    ) -> Vec<CustomerConnectionRow> {
        let mut conn = self
            .pool()
            .get()
            .await
            .expect("couldn't get db connection from pool");

        CustomerConnectionRow::list_connections_by_customer_id(
            &mut conn,
            &ids::TENANT_ID,
            &customer_id,
        )
        .await
        .expect("Failed to list connections")
    }
}
