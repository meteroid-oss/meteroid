//! Payment test helpers.
//!
//! Provides TestEnv methods for seeding and querying payment-related test data.
//! The actual seed logic lives in `data/payment.rs`.

use common_domain::ids::{ConnectorId, CustomerConnectionId, CustomerPaymentMethodId};
use diesel_models::customer_connection::CustomerConnectionRow;

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
}
