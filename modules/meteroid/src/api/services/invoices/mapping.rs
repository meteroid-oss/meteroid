pub mod invoices {
    use crate::api::services::shared::mapping::datetime::offset_datetime_to_timestamp;

    use meteroid_grpc::meteroid::api::invoices::v1::{
        DetailedInvoice, Invoice, InvoiceStatus, InvoicingProvider,
    };
    use meteroid_repository::invoices::{
        DetailedInvoice as DbDetailedInvoice, ListInvoice as DbListInvoice,
    };

    fn status_db_to_server(e: meteroid_repository::InvoiceStatusEnum) -> InvoiceStatus {
        match e {
            meteroid_repository::InvoiceStatusEnum::FINALIZED => InvoiceStatus::Finalized,
            meteroid_repository::InvoiceStatusEnum::PENDING => InvoiceStatus::Pending,
            meteroid_repository::InvoiceStatusEnum::DRAFT => InvoiceStatus::Draft,
            meteroid_repository::InvoiceStatusEnum::VOID => InvoiceStatus::Void,
        }
    }

    pub fn status_server_to_db(
        status: Option<i32>,
    ) -> Option<meteroid_repository::InvoiceStatusEnum> {
        status.and_then(|status_int| {
            InvoiceStatus::try_from(status_int)
                .ok()
                .map(|status| match status {
                    InvoiceStatus::Draft => meteroid_repository::InvoiceStatusEnum::DRAFT,
                    InvoiceStatus::Finalized => meteroid_repository::InvoiceStatusEnum::FINALIZED,
                    InvoiceStatus::Pending => meteroid_repository::InvoiceStatusEnum::PENDING,
                    InvoiceStatus::Void => meteroid_repository::InvoiceStatusEnum::VOID,
                })
        })
    }

    fn invoicing_provider_db_to_server(
        e: meteroid_repository::InvoicingProviderEnum,
    ) -> InvoicingProvider {
        match e {
            meteroid_repository::InvoicingProviderEnum::STRIPE => InvoicingProvider::Stripe,
        }
    }

    pub fn db_to_server(db_invoice: DbDetailedInvoice) -> DetailedInvoice {
        DetailedInvoice {
            id: db_invoice.id.to_string(),
            status: status_db_to_server(db_invoice.status).into(),
            invoicing_provider: invoicing_provider_db_to_server(db_invoice.invoicing_provider)
                .into(),
            created_at: Some(offset_datetime_to_timestamp(db_invoice.created_at)),
            updated_at: db_invoice.updated_at.map(offset_datetime_to_timestamp),
            invoice_date: db_invoice.invoice_date.to_string(),
            customer_id: db_invoice.customer_id.to_string(),
            customer_name: db_invoice.customer_name,
            plan_name: db_invoice.plan_name,
            plan_version: db_invoice.plan_version,
            plan_external_id: db_invoice.plan_external_id,
            subscription_id: db_invoice.subscription_id.to_string(),
            currency: db_invoice.currency,
            days_until_due: db_invoice.days_until_due,
            issued: db_invoice.issued,
            issue_attempts: db_invoice.issue_attempts,
        }
    }

    pub fn db_to_server_list(db_invoice: DbListInvoice) -> Invoice {
        Invoice {
            id: db_invoice.id.to_string(),
            status: status_db_to_server(db_invoice.status).into(),
            invoicing_provider: invoicing_provider_db_to_server(db_invoice.invoicing_provider)
                .into(),
            invoice_date: db_invoice.invoice_date.to_string(),
            customer_id: db_invoice.customer_id.to_string(),
            customer_name: db_invoice.customer_name,
            subscription_id: db_invoice.subscription_id.to_string(),
            currency: db_invoice.currency,
            days_until_due: db_invoice.days_until_due,
        }
    }
}
