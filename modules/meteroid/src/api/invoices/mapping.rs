pub mod invoices {
    use crate::api::customers::mapping::customer::ServerAddressWrapper;
    use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::invoices::v1::{
        DetailedInvoice, InlineCustomer, Invoice, InvoiceStatus, InvoiceType, InvoicingProvider,
        LineItem,
    };
    use meteroid_store::domain;
    use meteroid_store::domain::invoice_lines as domain_invoice_lines;
    use meteroid_store::errors::StoreError;

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
            domain::enums::InvoicingProviderEnum::Manual => InvoicingProvider::Manual,
        }
    }

    fn invoicing_type_domain_to_server(value: domain::enums::InvoiceType) -> InvoiceType {
        match value {
            domain::enums::InvoiceType::Recurring => InvoiceType::Recurring,
            domain::enums::InvoiceType::OneOff => InvoiceType::OneOff,
            domain::enums::InvoiceType::UsageThreshold => InvoiceType::UsageThreshold,
            domain::enums::InvoiceType::Adjustment => InvoiceType::Adjustment,
        }
    }

    pub fn domain_invoice_with_plan_details_to_server(
        value: domain::DetailedInvoice,
    ) -> error_stack::Result<DetailedInvoice, StoreError> {
        let domain::DetailedInvoice { invoice, .. } = value;

        let line_items: Vec<LineItem> = invoice.line_items.into_iter()
            .map(|line| {
                LineItem {
                    id: line.local_id,
                    name: line.name,
                    subtotal: line.subtotal,
                    metric_id: line.metric_id.as_proto(),
                    price_component_id: line.price_component_id.as_proto(),
                    end_date: line.end_date.as_proto(),
                    start_date: line.start_date.as_proto(),
                    quantity: line.quantity.as_proto(),
                    total: line.total,
                    unit_price: line.unit_price.as_proto(),
                    is_prorated: line.is_prorated,
                    product_id: line.product_id.as_proto(),
                    description: line.description,
                    sub_line_items: line.sub_lines.into_iter().map(
                        |sub_line| {
                            let attributes = match sub_line.attributes {
                                Some(domain_invoice_lines::SubLineAttributes::Package { raw_usage }) => {
                                    Some(meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::SublineAttributes::Package(
                                        meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::Package {
                                            raw_usage: raw_usage.as_proto()
                                        }
                                    ))
                                }
                                Some(domain_invoice_lines::SubLineAttributes::Tiered { first_unit, last_unit, flat_cap, flat_fee }) => {
                                    Some(meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::SublineAttributes::Tiered(
                                        meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::TieredOrVolume {
                                            first_unit: first_unit,
                                            last_unit: last_unit,
                                            flat_cap: flat_cap.as_proto(),
                                            flat_fee: flat_fee.as_proto(),
                                        }
                                    ))
                                }
                                Some(domain_invoice_lines::SubLineAttributes::Volume { first_unit, last_unit, flat_cap, flat_fee }) => {
                                    Some(meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::SublineAttributes::Volume(
                                        meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::TieredOrVolume {
                                            first_unit: first_unit,
                                            last_unit: last_unit,
                                            flat_cap: flat_cap.as_proto(),
                                            flat_fee: flat_fee.as_proto(),
                                        }
                                    ))
                                }
                                Some(domain_invoice_lines::SubLineAttributes::Matrix { dimension1_key, dimension1_value, dimension2_key, dimension2_value }) => {
                                    Some(meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::SublineAttributes::Matrix(
                                        meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::Matrix {
                                            dimension1_key: dimension1_key.clone(),
                                            dimension1_value: dimension1_value.clone(),
                                            dimension2_key: dimension2_key.clone(),
                                            dimension2_value: dimension2_value.clone(),
                                        }
                                    ))
                                }
                                None => None
                            };

                            meteroid_grpc::meteroid::api::invoices::v1::SubLineItem {
                                id: sub_line.local_id.clone(),
                                name: sub_line.name.clone(),
                                total: sub_line.total,
                                quantity: sub_line.quantity.as_proto(),
                                unit_price: sub_line.unit_price.as_proto(),
                                subline_attributes: attributes,
                            }
                        }
                    ).collect(),
                }
            })
            .collect();

        Ok(DetailedInvoice {
            id: invoice.id.as_proto(),
            status: status_domain_to_server(invoice.status).into(),
            created_at: invoice.created_at.as_proto(),
            updated_at: invoice.updated_at.as_proto(),
            tenant_id: invoice.tenant_id.as_proto(),
            customer_id: invoice.customer_id.as_proto(),
            subscription_id: invoice.subscription_id.as_proto(),
            currency: invoice.currency,
            external_invoice_id: invoice.external_invoice_id,
            invoice_number: invoice.invoice_number,
            invoicing_provider: invoicing_provider_domain_to_server(invoice.invoicing_provider)
                .into(),
            issued: invoice.issued,
            issue_attempts: invoice.issue_attempts,
            last_issue_attempt_at: invoice.last_issue_attempt_at.as_proto(),
            last_issue_error: invoice.last_issue_error,
            data_updated_at: invoice.data_updated_at.as_proto(),
            invoice_date: invoice.invoice_date.as_proto(),
            plan_version_id: invoice.plan_version_id.as_proto(),
            invoice_type: invoicing_type_domain_to_server(invoice.invoice_type).into(),
            finalized_at: invoice.finalized_at.as_proto(),
            subtotal: invoice.subtotal,
            subtotal_recurring: invoice.subtotal_recurring,
            tax_rate: invoice.tax_rate,
            tax_amount: invoice.tax_amount,
            total: invoice.total,
            amount_due: invoice.amount_due,
            net_terms: invoice.net_terms,
            reference: invoice.reference,
            memo: invoice.memo,
            local_id: invoice.local_id,
            due_at: invoice.due_at.as_proto(),
            plan_name: invoice.plan_name,
            customer_details: Some(InlineCustomer {
                id: invoice.customer_details.id.as_proto(),
                name: invoice.customer_details.name,
                snapshot_at: invoice.customer_details.snapshot_at.as_proto(),
                billing_address: invoice
                    .customer_details
                    .billing_address
                    .map(ServerAddressWrapper::try_from)
                    .transpose()?
                    .map(|x: ServerAddressWrapper| x.0),
            }),
            line_items,
        })
    }

    pub fn domain_to_server(value: domain::InvoiceWithCustomer) -> Invoice {
        Invoice {
            id: value.invoice.id.to_string(),
            invoice_number: value.invoice.invoice_number,
            status: status_domain_to_server(value.invoice.status).into(),
            invoicing_provider: invoicing_provider_domain_to_server(
                value.invoice.invoicing_provider,
            )
            .into(),
            invoice_date: value.invoice.invoice_date.to_string(),
            customer_id: value.invoice.customer_id.to_string(),
            customer_name: value.customer.name.to_string(),
            subscription_id: value.invoice.subscription_id.map(|x| x.to_string()),
            currency: value.invoice.currency,
            due_at: value.invoice.due_at.as_proto(),
            total: value.invoice.total,
        }
    }
}
