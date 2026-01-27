//! Test environment setup and fixtures.

use std::sync::Arc;

use rstest::fixture;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{BaseId, StoredDocumentId, SubscriptionId};
use diesel_models::subscriptions::SubscriptionRow;
use meteroid::workers::pgmq::processors::{
    run_once_invoice_orchestration, run_once_outbox_dispatch, run_once_payment_request,
};
use meteroid_mailer::service::MockMailerService;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{Invoice, OrderByRequest, PaginationRequest};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::store::PgConn;
use meteroid_store::{Services, Store};

use crate::data::ids::TENANT_ID;

/// Test environment containing all setup components.
///
/// This provides access to the store, services, and other test infrastructure
/// with convenient helper methods.
pub struct TestEnv {
    pub setup: MeteroidSetup,
    pub mailer: Arc<MockMailerService>,
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

    /// Get a subscription row by ID.
    pub async fn get_subscription(&self, id: SubscriptionId) -> SubscriptionRow {
        let mut conn = self.conn().await;
        SubscriptionRow::get_subscription_by_id(&mut conn, &TENANT_ID, id)
            .await
            .expect("Failed to get subscription")
            .subscription
    }

    /// Get invoices for a subscription.
    pub async fn get_invoices(&self, subscription_id: SubscriptionId) -> Vec<Invoice> {
        self.store()
            .list_invoices(
                TENANT_ID,
                None,
                Some(subscription_id),
                None,
                None,
                OrderByRequest::DateAsc,
                PaginationRequest {
                    page: 0,
                    per_page: None,
                },
            )
            .await
            .expect("Failed to list invoices")
            .items
            .into_iter()
            .map(|i| i.invoice)
            .collect()
    }

    /// Run the full billing pipeline:
    /// 1. Outbox dispatch (sends events to queues)
    /// 2. Invoice orchestration (handles InvoiceFinalized, requests PDF)
    /// 3. Simulate PDF generation for finalized invoices
    /// 4. Outbox dispatch again (for PDF generated events)
    /// 5. Invoice orchestration again (handles PDF generated, may send PaymentRequest)
    /// 6. Payment request processing (charges invoices)
    pub async fn run_outbox_and_orchestration(&self) {
        let store = Arc::new(self.store().clone());
        let services = Arc::new(self.services().clone());

        // Step 1-2: Initial outbox dispatch and orchestration
        run_once_outbox_dispatch(store.clone()).await;
        run_once_invoice_orchestration(store.clone(), services.clone()).await;

        // Step 3: Simulate PDF generation for finalized invoices without PDFs
        self.simulate_pdf_generation().await;

        // Step 4-5: Process the PDF generated events
        run_once_outbox_dispatch(store.clone()).await;
        run_once_invoice_orchestration(store.clone(), services.clone()).await;

        // Step 6: Process payment requests
        run_once_payment_request(store, services).await;
    }

    /// Simulate PDF generation for all finalized invoices that don't have a PDF yet.
    /// This calls save_invoice_documents which creates the invoice_pdf_generated outbox event.
    async fn simulate_pdf_generation(&self) {
        let invoices = self
            .store()
            .list_invoices(
                TENANT_ID,
                None,
                None,
                Some(InvoiceStatusEnum::Finalized),
                None,
                OrderByRequest::DateAsc,
                PaginationRequest {
                    page: 0,
                    per_page: None,
                },
            )
            .await
            .expect("Failed to list invoices");

        for invoice in invoices.items {
            // Skip invoices that already have a PDF
            if invoice.invoice.pdf_document_id.is_some() {
                continue;
            }

            // Generate a dummy PDF ID and save it
            let dummy_pdf_id = StoredDocumentId::new();
            self.store()
                .save_invoice_documents(
                    invoice.invoice.id,
                    invoice.invoice.tenant_id,
                    invoice.invoice.customer_id,
                    dummy_pdf_id,
                    None,
                )
                .await
                .expect("Failed to save invoice documents");
        }
    }

    /// Process all pending cycle transitions and due events.
    pub async fn process_cycles(&self) {
        self.services()
            .get_and_process_cycle_transitions()
            .await
            .expect("Failed to process cycle transitions");
        self.services()
            .get_and_process_due_events()
            .await
            .expect("Failed to process due events");
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

    TestEnv { setup, mailer }
}
