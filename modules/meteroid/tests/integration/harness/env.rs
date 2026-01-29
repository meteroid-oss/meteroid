//! Test environment setup and fixtures.

use std::sync::Arc;

use rstest::fixture;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use meteroid_mailer::service::MockMailerService;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::store::PgConn;
use meteroid_store::{Services, Store};

/// Test environment containing all setup components.
///
/// This provides access to the store, services, and other test infrastructure.
/// Domain-specific helper methods are implemented in separate modules:
/// - `coupons.rs` - Coupon creation and management
/// - `subscriptions.rs` - Subscription queries
/// - `invoices.rs` - Invoice queries
/// - `billing.rs` - Billing pipeline processing
pub struct TestEnv {
    pub setup: MeteroidSetup,
    pub _mailer: Arc<MockMailerService>,
}

impl TestEnv {
    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &meteroid_store::store::PgPool {
        &self.setup.store.pool
    }

    /// Get a reference to the services.
    pub fn services(&self) -> &Services {
        &self.setup.services
    }

    /// Get a reference to the store.
    pub fn store(&self) -> &Store {
        &self.setup.store
    }

    /// Get a database connection from the pool.
    pub async fn conn(&self) -> PgConn {
        self.pool().get().await.expect("Failed to get connection")
    }

    /// Seed the mock payment provider.
    ///
    /// # Arguments
    /// * `fail_payment_intent` - If true, payments will fail
    pub async fn seed_mock_payment_provider(&self, fail_payment_intent: bool) {
        crate::data::minimal::run_mock_payment_provider_seed(self.pool(), fail_payment_intent)
            .await;
    }

    /// Seed customer payment methods (Uber & Spotify).
    pub async fn seed_customer_payment_methods(&self) {
        crate::data::minimal::run_customer_payment_methods_seed(self.pool()).await;
    }

    /// Seed both payment provider and customer payment methods.
    pub async fn seed_payments(&self) {
        self.seed_mock_payment_provider(false).await;
        self.seed_customer_payment_methods().await;
    }
}

/// Create a test environment with PLANS seed level.
///
/// This is the default fixture for most subscription tests.
#[fixture]
pub async fn test_env() -> TestEnv {
    test_env_with_seed(SeedLevel::PLANS).await
}

/// Create a test environment with minimal seed (no plans).
#[fixture]
pub async fn test_env_minimal() -> TestEnv {
    test_env_with_seed(SeedLevel::MINIMAL).await
}

/// Create a test environment with a specific seed level.
///
/// Uses a shared Postgres container with database templating for fast test setup.
/// Migrations run once on the template; each test gets a fresh database copy.
pub async fn test_env_with_seed(seed_level: SeedLevel) -> TestEnv {
    helpers::init::logging();

    // Create a new database from the shared template (migrations already applied)
    let postgres_connection_string = meteroid_it::container::create_test_database().await;

    let mailer = Arc::new(MockMailerService::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        seed_level,
        Arc::new(MockUsageClient::noop()),
        mailer.clone(),
    )
    .await;

    TestEnv {
        setup,
        _mailer: mailer,
    }
}
