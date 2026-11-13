//! Test environment setup and fixtures.

use std::sync::Arc;

use rstest::fixture;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{BaseId, InvoiceId};
use meteroid_store::clients::usage::{MockUsageClient, UsageClient};
use meteroid_store::store::PgConn;
use meteroid_store::{Services, Store};

use super::CapturingMailer;

/// Test environment containing all setup components.
///
/// This provides access to the store, services, and other test infrastructure.
/// Domain-specific helper methods are implemented in separate modules:
/// - `billing.rs` - Billing pipeline processing
/// - `coupons.rs` - Coupon creation and management
/// - `emails.rs` - Email worker draining
/// - `invoices.rs` - Invoice queries
/// - `payments.rs` - Payment provider and payment method helpers
/// - `subscriptions.rs` - Subscription queries
pub struct TestEnv {
    pub setup: MeteroidSetup,
    pub mailer: Arc<CapturingMailer>,
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

    /// Sets the seeded tenant's delivery mode in the database: the seeded tenant is a
    /// Development workspace, which the store no longer lets go `Live`.
    pub async fn set_email_delivery_mode(&self, mode: diesel_models::enums::EmailDeliveryModeEnum) {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::tenant::dsl as t;

        let mut conn = self.conn().await;
        diesel::update(t::tenant.filter(t::id.eq(crate::data::ids::TENANT_ID)))
            .set(t::email_delivery_mode.eq(mode))
            .execute(&mut conn)
            .await
            .expect("set email delivery mode");
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
    test_env_with_seed_and_usage(seed_level, Arc::new(MockUsageClient::noop())).await
}

/// A test environment serving the edition `modules` (EE module tests).
pub async fn test_env_with_modules(
    seed_level: SeedLevel,
    modules: meteroid::module::Modules,
) -> TestEnv {
    test_env_with_modules_and_usage(seed_level, Arc::new(MockUsageClient::noop()), modules).await
}

/// Like [`test_env_with_modules`], reading usage through `usage_client`.
pub async fn test_env_with_modules_and_usage(
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
    modules: meteroid::module::Modules,
) -> TestEnv {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(seed_level).await;
    let mailer = Arc::new(CapturingMailer::new());
    let setup = meteroid_it::container::start_meteroid_with_clients_and_modules(
        database,
        seed_level,
        usage_client,
        mailer.clone(),
        modules,
    )
    .await;
    TestEnv { setup, mailer }
}

/// Create a test environment with a specific seed level and custom usage client.
///
/// Use this when you need to inject pre-populated usage data (e.g. for testing
/// arrear/usage-based billing during cancellation).
pub async fn test_env_with_seed_and_usage(
    seed_level: SeedLevel,
    usage_client: Arc<dyn UsageClient>,
) -> TestEnv {
    helpers::init::logging();

    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(seed_level).await;

    let mailer = Arc::new(CapturingMailer::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        seed_level,
        usage_client,
        mailer.clone(),
    )
    .await;

    TestEnv { setup, mailer }
}

/// Create a test environment with PLANS seed and a custom usage client.
pub async fn test_env_with_usage(usage_client: Arc<dyn UsageClient>) -> TestEnv {
    helpers::init::logging();

    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;

    let mailer = Arc::new(CapturingMailer::new());

    let setup = meteroid_it::container::start_meteroid_with_clients(
        postgres_connection_string,
        SeedLevel::PLANS,
        usage_client,
        mailer.clone(),
    )
    .await;

    TestEnv { setup, mailer }
}

impl TestEnv {
    /// Skips the credit invariants checked when this test's database is dropped; see
    /// [`crate::meteroid_it::container::MeteroidSetup::skip_credit_invariants`].
    #[allow(dead_code)]
    pub fn skip_credit_invariants(&self, reason: &'static str) {
        self.setup.skip_credit_invariants(reason);
    }

    /// Compares the seeded tenant's invoices, credit notes and grants with `corpus/<name>.txt`.
    pub async fn assert_corpus(&self, name: &str) {
        let mut conn = self.conn().await;
        crate::corpus::assert_tenant(&mut conn, crate::data::ids::TENANT_ID.as_uuid(), name).await;
    }

    /// See [`crate::meteroid_it::container::MeteroidSetup::skip_checks`].
    #[allow(dead_code)]
    pub fn skip_checks(&self, checks: &[&'static str], reason: &'static str) {
        self.setup.skip_checks(checks, reason);
    }

    /// See [`crate::meteroid_it::container::MeteroidSetup::exempt_invoice_from`].
    #[allow(dead_code)]
    pub fn exempt_invoice_from(
        &self,
        check: &'static str,
        invoice: InvoiceId,
        reason: &'static str,
    ) {
        self.setup.exempt_invoice_from(check, invoice, reason);
    }
}
