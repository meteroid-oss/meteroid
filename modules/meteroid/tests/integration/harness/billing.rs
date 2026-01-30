//! Billing pipeline test helpers.

use std::sync::Arc;

use common_domain::ids::{BaseId, StoredDocumentId};
use meteroid::workers::pgmq::processors::{
    run_once_invoice_orchestration, run_once_outbox_dispatch, run_once_payment_request,
};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{OrderByRequest, PaginationRequest};
use meteroid_store::repositories::InvoiceInterface;

use crate::data::ids::TENANT_ID;

use super::TestEnv;

impl TestEnv {
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
