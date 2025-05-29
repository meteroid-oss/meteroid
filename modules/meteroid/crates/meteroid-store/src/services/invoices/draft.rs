use crate::domain::{
    Customer, InlineCustomer, InlineInvoicingEntity, Invoice, InvoiceNew, InvoiceStatusEnum,
    InvoiceTotals, InvoiceTotalsParams, InvoiceType, SubscriptionDetails,
};
use crate::errors::StoreError;
use crate::repositories::invoices::insert_invoice_tx;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::services::Services;
use crate::store::PgConn;
use chrono::NaiveTime;
use common_domain::ids::TenantId;
use error_stack::ResultExt;

impl Services {
    pub(in crate::services) async fn create_subscription_draft_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_details: &SubscriptionDetails,
        customer: Customer,
    ) -> error_stack::Result<Option<Invoice>, StoreError> {
        let subscription = &subscription_details.subscription;

        let invoice_date = subscription.current_period_start;

        // Compute invoice lines for the period
        let invoice_lines = self
            .compute_invoice_lines(conn, &invoice_date, subscription_details)
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        if invoice_lines.is_empty() {
            log::info!(
                "No invoice lines computed for subscription {}. Skipping draft invoice creation.",
                subscription.id
            );
            return Ok(None);
        }

        let totals = InvoiceTotals::from_params(InvoiceTotalsParams {
            line_items: &invoice_lines,
            total: 0, // "no prepaid"  TODO ?
            amount_due: 0,
            tax_rate: 0,
            customer_balance_cents: customer.balance_value_cents,
            subscription_applied_coupons: &vec![], // TODO
            invoice_currency: subscription.currency.as_str(),
        });

        let due_date = (invoice_date + chrono::Duration::days(subscription.net_terms as i64))
            .and_time(NaiveTime::MIN);

        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        // Draft invoice uses "draft" as invoice number
        let invoice_number = "draft";

        let invoice_new = InvoiceNew {
            tenant_id: subscription.tenant_id,
            customer_id: subscription.customer_id,
            subscription_id: Some(subscription.id),
            plan_version_id: Some(subscription.plan_version_id),
            invoice_type: InvoiceType::Recurring,
            currency: subscription.currency.clone(),
            external_invoice_id: None,
            line_items: invoice_lines,
            issued: false,
            issue_attempts: 0,
            last_issue_attempt_at: None,
            last_issue_error: None,
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            external_status: None, // TODO
            invoice_date,
            finalized_at: None,
            total: totals.total,
            amount_due: totals.amount_due,
            net_terms: subscription.net_terms as i32,
            subtotal: totals.subtotal,
            subtotal_recurring: totals.subtotal_recurring,
            tax_amount: totals.tax_amount,
            tax_rate: 0, // TODO
            reference: None,
            memo: None,
            due_at: Some(due_date),
            plan_name: Some(subscription.plan_name.clone()),
            invoice_number: invoice_number.to_string(),
            customer_details: InlineCustomer {
                id: subscription.customer_id,
                name: customer.name.clone(),
                billing_address: customer.billing_address.clone(),
                vat_number: customer.vat_number.clone(),
                email: customer.billing_email.clone(),
                alias: customer.alias.clone(),
                snapshot_at: chrono::Utc::now().naive_utc(),
            },
            seller_details: InlineInvoicingEntity {
                id: invoicing_entity.id,
                legal_name: invoicing_entity.legal_name.clone(),
                vat_number: invoicing_entity.vat_number.clone(),
                address: invoicing_entity.address(),
                snapshot_at: chrono::Utc::now().naive_utc(),
            },
        };

        let inserted_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        Ok(Some(inserted_invoice))
    }
}
