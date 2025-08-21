use crate::domain::{
    Customer, InlineCustomer, InlineInvoicingEntity, Invoice, InvoiceNew, InvoicePaymentStatus,
    InvoiceStatusEnum, InvoiceType, LineItem, SubscriptionDetails,
};
use crate::errors::StoreError;
use crate::repositories::invoices::insert_invoice_tx;
use crate::repositories::invoicing_entities::InvoicingEntityInterface;
use crate::services::Services;
use crate::store::PgConn;
use chrono::{NaiveDate, NaiveTime};
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
        let invoice_content = self
            .compute_invoice(conn, &invoice_date, subscription_details, None)
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        if invoice_content.invoice_lines.is_empty() {
            log::info!(
                "No invoice lines computed for subscription {}. Skipping draft invoice creation.",
                subscription.id
            );
            return Ok(None);
        }

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
            line_items: invoice_content.invoice_lines,
            coupons: invoice_content.applied_coupons,
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            invoice_date,
            finalized_at: None,
            total: invoice_content.total,
            amount_due: invoice_content.amount_due,
            net_terms: subscription.net_terms as i32,
            subtotal: invoice_content.subtotal,
            subtotal_recurring: invoice_content.subtotal_recurring,
            reference: None,
            purchase_order: None,
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
            auto_advance: true,
            payment_status: InvoicePaymentStatus::Unpaid,
            discount: invoice_content.discount,
            tax_breakdown: invoice_content.tax_breakdown,
            tax_amount: invoice_content.tax_amount,
        };

        let inserted_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        Ok(Some(inserted_invoice))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::services) async fn create_oneoff_draft_invoice(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoice_date: NaiveDate,
        invoice_lines: Vec<LineItem>,
        customer: &Customer,
        currency: String,
        discount: Option<u64>,
        prepaid_amount: Option<u64>,
    ) -> error_stack::Result<Option<Invoice>, StoreError> {
        let invoicing_entity = self
            .store
            .get_invoicing_entity(tenant_id, Some(customer.invoicing_entity_id))
            .await?;

        let invoice_content = self
            .compute_oneoff_invoice(
                conn,
                &invoice_date,
                invoice_lines,
                &invoicing_entity,
                customer,
                currency.clone(),
                discount,
                prepaid_amount,
            )
            .await
            .change_context(StoreError::InvoiceComputationError)?;

        let due_date = (invoice_date + chrono::Duration::days(invoicing_entity.net_terms as i64))
            .and_time(NaiveTime::MIN);

        // Draft invoice uses "draft" as invoice number
        let invoice_number = "draft";

        let invoice_new = InvoiceNew {
            tenant_id: customer.tenant_id,
            customer_id: customer.id,
            subscription_id: None,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            currency,
            line_items: invoice_content.invoice_lines,
            coupons: invoice_content.applied_coupons,
            data_updated_at: None,
            status: InvoiceStatusEnum::Draft,
            invoice_date,
            finalized_at: None,
            total: invoice_content.total,
            amount_due: invoice_content.amount_due,
            net_terms: invoicing_entity.net_terms,
            subtotal: invoice_content.subtotal,
            subtotal_recurring: invoice_content.subtotal_recurring,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: Some(due_date),
            plan_name: None,
            invoice_number: invoice_number.to_string(),
            customer_details: InlineCustomer {
                id: customer.id,
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
            auto_advance: true,
            payment_status: InvoicePaymentStatus::Unpaid,
            discount: invoice_content.discount,
            tax_breakdown: invoice_content.tax_breakdown,
            tax_amount: invoice_content.tax_amount,
        };

        let inserted_invoice = insert_invoice_tx(&self.store, conn, invoice_new).await?;

        Ok(Some(inserted_invoice))
    }
}
