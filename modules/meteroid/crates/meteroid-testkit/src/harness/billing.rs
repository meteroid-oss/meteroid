//! Billing pipeline test helpers.

use std::sync::Arc;

use bytes::Bytes;
use meteroid::services::storage::ObjectKind;
use meteroid::workers::pgmq::processors::{
    run_once_invoice_orchestration, run_once_payment_request,
};
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::repositories::InvoiceInterface;

use crate::data::ids::TENANT_ID;

use super::TestEnv;

/// The smallest PDF that parsers accept, so email attachments carry real bytes.
const PLACEHOLDER_PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

impl TestEnv {
    /// Run the full billing pipeline. Producers fan events out to consumer queues via
    /// topic routing at publish time, so this just drives the consumers:
    /// 1. Invoice orchestration (handles InvoiceFinalized, requests PDF)
    /// 2. Simulate PDF generation for finalized invoices
    /// 3. Invoice orchestration again (handles PDF generated, may send PaymentRequest)
    /// 4. Payment request processing (charges invoices)
    /// 5. Invoice orchestration again (PaymentTransactionSettled → InvoicePaid → transitions)
    pub async fn run_outbox_and_orchestration(&self) {
        let store = Arc::new(self.store().clone());
        let services = Arc::new(self.services().clone());

        // Initial orchestration (InvoiceFinalized → request PDF)
        run_once_invoice_orchestration(store.clone(), services.clone()).await;

        // Simulate PDF generation for finalized invoices without PDFs
        self.simulate_pdf_generation().await;

        // Process the PDF generated events
        run_once_invoice_orchestration(store.clone(), services.clone()).await;

        // Process payment requests
        run_once_payment_request(store.clone(), services.clone()).await;

        // Process PaymentTransactionSettled events (marks invoice paid, emits InvoicePaid)
        run_once_invoice_orchestration(store.clone(), services.clone()).await;

        // Process InvoicePaid events (triggers TrialExpired → Active transition)
        run_once_invoice_orchestration(store, services).await;
    }

    /// Simulate PDF generation for all finalized invoices that don't have a PDF yet.
    /// Stores a placeholder blob so the email worker can attach it, then calls
    /// save_invoice_documents which creates the invoice_pdf_generated outbox event.
    async fn simulate_pdf_generation(&self) {
        let invoices = self
            .store()
            .list_invoices(
                TENANT_ID,
                None,
                None,
                Some(InvoiceStatusEnum::Finalized),
                None,
                None,
                false,
                None,
                vec![],
                Some("created_at.asc".to_string()),
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

            let pdf = self
                .setup
                .object_store
                .store(
                    ObjectKind::InvoicePdf,
                    invoice.invoice.tenant_id,
                    Bytes::from_static(PLACEHOLDER_PDF),
                )
                .await
                .expect("Failed to store placeholder PDF");
            self.store()
                .save_invoice_documents(
                    invoice.invoice.id,
                    invoice.invoice.tenant_id,
                    invoice.invoice.customer_id,
                    pdf,
                    None,
                    None,
                )
                .await
                .expect("Failed to save invoice documents");
        }
    }

    /// Process all pending cycle transitions and due events.
    /// Moves the present to `at` (see `common_domain::clock`); time keeps passing from there.
    pub fn set_now(&self, at: chrono::DateTime<chrono::Utc>) {
        common_domain::clock::test::set(at);
    }

    /// Moves the present forward by `by`.
    pub fn advance(&self, by: chrono::Duration) {
        common_domain::clock::test::advance(by);
    }

    /// Lives through every day until `until`, as the workers would: each day the clock is moved
    /// to it, then due cycles, scheduled events and their outbox work run until nothing is left.
    pub async fn run_until(&self, until: chrono::NaiveDate) {
        let mut day = common_domain::clock::now().date_naive();
        while day < until {
            day = day.succ_opt().expect("a next day");
            self.set_now(day.and_hms_opt(1, 0, 0).expect("1:00").and_utc());
            for _ in 0..50 {
                let cycles = self
                    .services()
                    .get_and_process_cycle_transitions()
                    .await
                    .expect("Failed to process cycle transitions")
                    .processed;
                let events = self
                    .services()
                    .get_and_process_due_events()
                    .await
                    .expect("Failed to process due events");
                self.run_outbox_and_orchestration().await;
                if cycles == 0 && events == 0 {
                    break;
                }
            }
        }
    }

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

    /// Process only due scheduled events (e.g. grace-period invoice finalization).
    #[allow(dead_code)]
    pub async fn process_due_events(&self) {
        self.services()
            .get_and_process_due_events()
            .await
            .expect("Failed to process due events");
    }

    /// Process only pending cycle transitions (renewals/activations), WITHOUT firing due
    /// scheduled events. Lets a test observe a freshly-created draft (and its scheduled
    /// finalization) before finalization actually runs.
    #[allow(dead_code)]
    pub async fn process_cycle_transitions(&self) {
        self.services()
            .get_and_process_cycle_transitions()
            .await
            .expect("Failed to process cycle transitions");
    }
}
