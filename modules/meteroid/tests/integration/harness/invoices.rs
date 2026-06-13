//! Invoice test helpers.

use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId};
use meteroid_store::StoreResult;
use meteroid_store::domain::customers::CustomerTopUpBalance;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::{DetailedInvoice, Invoice, InvoicingEntityPatch, PaginationRequest};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{CustomersInterface, InvoiceInterface};

use crate::data::ids::{INVOICING_ENTITY_ID, TENANT_ID, USER_ID};

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
                Some("created_at.asc".to_string()),
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

    /// Get all (listed) invoices for a customer. Like the product UI, this excludes
    /// consolidated child drafts that have been merged into a parent invoice.
    pub async fn get_customer_invoices(&self, customer_id: CustomerId) -> Vec<Invoice> {
        self.store()
            .list_invoices(
                TENANT_ID,
                Some(customer_id),
                None,
                None,
                None,
                Some("created_at.asc".to_string()),
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

    /// Enable (or disable) invoice consolidation on the test tenant's invoicing entity.
    ///
    /// consolidate_recurring_invoices is enterprise-only and not settable through the OSS
    /// domain API, so the test flips the database column directly.
    pub async fn set_consolidate_recurring_invoices(&self, enabled: bool) {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::invoicing_entity::dsl as ie;

        let mut conn = self.conn().await;
        diesel::update(ie::invoicing_entity.filter(ie::id.eq(INVOICING_ENTITY_ID)))
            .set(ie::consolidate_recurring_invoices.eq(enabled))
            .execute(&mut conn)
            .await
            .expect("Failed to toggle consolidation flag");
    }

    /// Set the consolidation flag together with a grace period on the test invoicing entity.
    pub async fn set_consolidate_and_grace(&self, enabled: bool, grace_period_hours: i32) {
        // Grace period via the domain API; the consolidation flag is enterprise-only and set
        // directly on the database column.
        self.store()
            .patch_invoicing_entity(
                Actor::User { id: USER_ID },
                InvoicingEntityPatch {
                    id: INVOICING_ENTITY_ID,
                    grace_period_hours: Some(grace_period_hours),
                    ..Default::default()
                },
                TENANT_ID,
            )
            .await
            .expect("Failed to set grace period");
        self.set_consolidate_recurring_invoices(enabled).await;
    }

    /// Get the per-subscription child invoices merged into a consolidated parent invoice.
    pub async fn get_consolidated_children(&self, parent_invoice_id: InvoiceId) -> Vec<Invoice> {
        self.store()
            .list_consolidated_children(TENANT_ID, parent_invoice_id)
            .await
            .expect("Failed to list consolidated children")
    }

    /// Add prepaid credit (in the customer's currency) to a customer's balance.
    pub async fn top_up_balance(&self, customer_id: CustomerId, cents: i64) {
        self.store()
            .top_up_customer_balance(CustomerTopUpBalance {
                created_by: *USER_ID,
                tenant_id: TENANT_ID,
                customer_id,
                cents,
                notes: None,
            })
            .await
            .expect("Failed to top up customer balance");
    }

    /// Attempt to void an invoice, returning the raw result so callers can assert success/failure.
    pub async fn try_void_invoice(&self, invoice_id: InvoiceId) -> StoreResult<DetailedInvoice> {
        self.store()
            .void_invoice(Actor::User { id: USER_ID }, invoice_id, TENANT_ID)
            .await
    }

    /// Get detailed invoice including transactions.
    pub async fn get_detailed_invoice(&self, invoice_id: InvoiceId) -> DetailedInvoice {
        let detailed = self
            .store()
            .get_detailed_invoice_by_id(TENANT_ID, invoice_id)
            .await
            .expect("Failed to get detailed invoice");

        let transactions = self
            .store()
            .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
            .await
            .expect("Failed to list payment transactions");

        let domain_transactions = transactions.into_iter().map(|t| t.transaction).collect();

        detailed.with_transactions(domain_transactions)
    }
}
