//! Invoice test helpers.

use common_domain::ids::{InvoiceId, SubscriptionId};
use meteroid_store::domain::{DetailedInvoice, Invoice, OrderByRequest, PaginationRequest};
use meteroid_store::repositories::InvoiceInterface;

use crate::data::ids::TENANT_ID;

use super::TestEnv;

impl TestEnv {
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

    /// Get detailed invoice including transactions.
    pub async fn get_detailed_invoice(&self, invoice_id: InvoiceId) -> DetailedInvoice {
        self.store()
            .get_detailed_invoice_by_id(TENANT_ID, invoice_id)
            .await
            .expect("Failed to get detailed invoice")
    }
}
