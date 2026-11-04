//! Invoice test helpers.

use chrono::NaiveDate;
use common_domain::ids::{CustomerId, InvoiceId, SubscriptionId};
use meteroid_store::StoreResult;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::{
    DetailedInvoice, Invoice, InvoiceNew, InvoicingEntityPatch, PaginationRequest,
};
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

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
            .expect("Failed to list invoices")
            .items
            .into_iter()
            .map(|i| i.invoice)
            .collect()
    }

    /// Enable (or disable) invoice consolidation on the test tenant's invoicing entity.
    pub async fn set_consolidate_recurring_invoices(&self, enabled: bool) {
        self.store()
            .patch_invoicing_entity(
                Actor::User { id: USER_ID },
                InvoicingEntityPatch {
                    id: INVOICING_ENTITY_ID,
                    consolidate_recurring_invoices: Some(enabled),
                    ..Default::default()
                },
                TENANT_ID,
            )
            .await
            .expect("Failed to toggle consolidation flag");
    }

    /// Set the consolidation flag together with a grace period on the test invoicing entity.
    pub async fn set_consolidate_and_grace(&self, enabled: bool, grace_period_hours: i32) {
        self.store()
            .patch_invoicing_entity(
                Actor::User { id: USER_ID },
                InvoicingEntityPatch {
                    id: INVOICING_ENTITY_ID,
                    consolidate_recurring_invoices: Some(enabled),
                    grace_period_hours: Some(grace_period_hours),
                    ..Default::default()
                },
                TENANT_ID,
            )
            .await
            .expect("Failed to set consolidation + grace");
    }

    /// Get the per-subscription child invoices merged into a consolidated parent invoice.
    pub async fn get_consolidated_children(&self, parent_invoice_id: InvoiceId) -> Vec<Invoice> {
        self.store()
            .list_consolidated_children(TENANT_ID, parent_invoice_id)
            .await
            .expect("Failed to list consolidated children")
    }

    /// Grant invoice credit in the customer's currency, spendable at once.
    pub async fn fund_invoice_credit(&self, customer_id: CustomerId, cents: i64) {
        crate::helpers::credit::fund(self.store(), TENANT_ID, customer_id, None, cents).await;
    }

    /// Invoice credit spendable now in the customer's currency, in minor units.
    pub async fn invoice_credit_balance_minor(&self, customer_id: CustomerId) -> i64 {
        crate::helpers::credit::available(self.store(), TENANT_ID, customer_id, None).await
    }

    /// Attempt to void an invoice, returning the raw result so callers can assert success/failure.
    pub async fn try_void_invoice(&self, invoice_id: InvoiceId) -> StoreResult<DetailedInvoice> {
        self.store()
            .void_invoice(Actor::User { id: USER_ID }, invoice_id, TENANT_ID)
            .await
    }

    /// Invoke the public (manual/API) finalize entry point — the "Finalize" action a console or the
    /// REST API triggers. A recurring subscription period draft is settled like the automated grace
    /// finalize (recompute usage, then finalize or `Close` an empty period); every other invoice
    /// finalizes from its stored lines.
    pub async fn manual_finalize_invoice(
        &self,
        invoice_id: InvoiceId,
    ) -> StoreResult<DetailedInvoice> {
        self.services()
            .finalize_invoice(Actor::User { id: USER_ID }, invoice_id, TENANT_ID)
            .await
    }

    /// Write a terminal (`Closed`) `Recurring` invoice for `subscription_id` at `invoice_date`,
    /// bypassing the billing pipeline.
    ///
    /// Deliberate fault injection. No supported flow leaves a subscription's period already
    /// settled *before* the checkout meant to bill it — that state comes from a partially applied
    /// prior run, a replayed settlement, or a manual repair. Callers that collect money before
    /// creating the invoice have to survive it anyway, so the tests write the precondition
    /// directly instead of pretending the pipeline can produce it.
    pub async fn inject_settled_period_invoice(
        &self,
        subscription_id: SubscriptionId,
        invoice_date: NaiveDate,
    ) -> Invoice {
        self.insert_subscription_invoice(subscription_id, invoice_date, InvoiceStatusEnum::Closed)
            .await
    }

    /// Write an empty `Draft` `Recurring` invoice for `subscription_id`, bypassing the
    /// billing pipeline — enough of an invoice to exercise draft-only operations.
    pub async fn insert_draft_invoice(
        &self,
        subscription_id: SubscriptionId,
        invoice_date: NaiveDate,
    ) -> Invoice {
        self.insert_subscription_invoice(subscription_id, invoice_date, InvoiceStatusEnum::Draft)
            .await
    }

    async fn insert_subscription_invoice(
        &self,
        subscription_id: SubscriptionId,
        invoice_date: NaiveDate,
        status: InvoiceStatusEnum,
    ) -> Invoice {
        let subscription = self.get_subscription(subscription_id).await;
        let customer = self.get_customer(subscription.customer_id).await;
        let invoicing_entity = self
            .store()
            .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
            .await
            .expect("Failed to get invoicing entity");

        self.store()
            .insert_invoice(InvoiceNew {
                status,
                tenant_id: TENANT_ID,
                customer_id: subscription.customer_id,
                subscription_id: Some(subscription_id),
                cycle_index: subscription.cycle_index.map(|c| c.max(0) as u32),
                currency: subscription.currency.clone(),
                invoice_number: "draft".to_string(),
                line_items: vec![],
                coupons: vec![],
                data_updated_at: None,
                invoice_date,
                billing_period_start: invoice_date,
                tax_inclusive: false,
                plan_version_id: Some(subscription.plan_version_id),
                invoice_type: InvoiceType::Recurring,
                finalized_at: None,
                subtotal: 0,
                subtotal_recurring: 0,
                discount: 0,
                tax_amount: 0,
                total: 0,
                amount_due: 0,
                applied_credits: 0,
                promotional_credits: 0,
                net_terms: subscription.net_terms,
                reference: None,
                purchase_order: None,
                memo: None,
                due_at: None,
                plan_name: None,
                customer_details: customer.into(),
                seller_details: invoicing_entity.into(),
                auto_advance: false,
                payment_status: InvoicePaymentStatus::Unpaid,
                tax_breakdown: vec![],
                manual: false,
                invoicing_entity_id: INVOICING_ENTITY_ID,
                parent_invoice_id: None,
                consolidated_into_invoice_id: None,
                annexes: vec![],
                custom_properties: serde_json::Value::Null,
            })
            .await
            .expect("Failed to insert invoice")
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

impl TestEnv {
    /// Advance `cycle_index` without running a renewal — a state the scheduler reaches on its own,
    /// injected directly so the next period's invoice doesn't obscure the assertion.
    pub async fn advance_subscription_cycle(&self, subscription_id: SubscriptionId, to: i32) {
        use diesel_async::RunQueryDsl;
        let mut conn = self.conn().await;
        diesel::sql_query("UPDATE subscription SET cycle_index = $1 WHERE id = $2")
            .bind::<diesel::sql_types::Integer, _>(to)
            .bind::<diesel::sql_types::Uuid, _>(*subscription_id)
            .execute(&mut conn)
            .await
            .expect("failed to advance subscription cycle_index");
    }

    /// Move a subscription's current billing cycle to `[start, end)` without running a renewal.
    pub async fn set_subscription_period(
        &self,
        subscription_id: SubscriptionId,
        start: NaiveDate,
        end: NaiveDate,
    ) {
        use diesel_async::RunQueryDsl;
        let mut conn = self.conn().await;
        diesel::sql_query(
            "UPDATE subscription SET current_period_start = $1, current_period_end = $2 WHERE id = $3",
        )
        .bind::<diesel::sql_types::Date, _>(start)
        .bind::<diesel::sql_types::Date, _>(end)
        .bind::<diesel::sql_types::Uuid, _>(*subscription_id)
        .execute(&mut conn)
        .await
        .expect("failed to set subscription period");
    }
}
