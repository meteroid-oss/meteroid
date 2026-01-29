//! Invoice test helpers.

use common_domain::ids::SubscriptionId;
use meteroid_store::domain::{Invoice, OrderByRequest, PaginationRequest};
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
}
