pub mod invoices {
    use crate::api::shared::mapping::datetime::chrono_to_timestamp;

    use meteroid_grpc::meteroid::api::invoices::v1::{
        DetailedInvoice, Invoice, InvoiceStatus, InvoicingProvider,
    };
    use meteroid_store::domain;

    fn status_domain_to_server(value: domain::enums::InvoiceStatusEnum) -> InvoiceStatus {
        match value {
            domain::enums::InvoiceStatusEnum::Finalized => InvoiceStatus::Finalized,
            domain::enums::InvoiceStatusEnum::Pending => InvoiceStatus::Pending,
            domain::enums::InvoiceStatusEnum::Draft => InvoiceStatus::Draft,
            domain::enums::InvoiceStatusEnum::Void => InvoiceStatus::Void,
        }
    }

    pub fn status_server_to_domain(
        status: Option<i32>,
    ) -> Option<domain::enums::InvoiceStatusEnum> {
        status.and_then(|status_int| {
            InvoiceStatus::try_from(status_int)
                .ok()
                .map(|status| match status {
                    InvoiceStatus::Draft => domain::enums::InvoiceStatusEnum::Draft,
                    InvoiceStatus::Finalized => domain::enums::InvoiceStatusEnum::Finalized,
                    InvoiceStatus::Pending => domain::enums::InvoiceStatusEnum::Pending,
                    InvoiceStatus::Void => domain::enums::InvoiceStatusEnum::Void,
                })
        })
    }

    fn invoicing_provider_domain_to_server(
        value: domain::enums::InvoicingProviderEnum,
    ) -> InvoicingProvider {
        match value {
            domain::enums::InvoicingProviderEnum::Stripe => InvoicingProvider::Stripe,
        }
    }

    pub fn domain_invoice_with_plan_details_to_server(
        value: domain::InvoiceWithPlanDetails,
    ) -> DetailedInvoice {
        DetailedInvoice {
            id: value.id.to_string(),
            status: status_domain_to_server(value.status).into(),
            invoicing_provider: invoicing_provider_domain_to_server(value.invoicing_provider)
                .into(),
            created_at: Some(chrono_to_timestamp(value.created_at)),
            updated_at: value.updated_at.map(chrono_to_timestamp),
            invoice_date: value.invoice_date.to_string(),
            customer_id: value.customer_id.to_string(),
            customer_name: value.customer_name,
            plan_name: value.plan_name,
            plan_version: value.plan_version,
            plan_external_id: value.plan_external_id,
            subscription_id: value.subscription_id.to_string(),
            currency: value.currency,
            days_until_due: value.days_until_due,
            issued: value.issued,
            issue_attempts: value.issue_attempts,
            amount_cents: value.amount_cents,
        }
    }

    pub fn domain_to_server(value: domain::InvoiceWithCustomer) -> Invoice {
        Invoice {
            id: value.invoice.id.to_string(),
            status: status_domain_to_server(value.invoice.status).into(),
            invoicing_provider: invoicing_provider_domain_to_server(
                value.invoice.invoicing_provider,
            )
            .into(),
            invoice_date: value.invoice.invoice_date.to_string(),
            customer_id: value.invoice.customer_id.to_string(),
            customer_name: value.customer.name.to_string(),
            subscription_id: value.invoice.subscription_id.to_string(),
            currency: value.invoice.currency,
            days_until_due: value.invoice.days_until_due,
            amount_cents: value.invoice.amount_cents,
        }
    }
}
