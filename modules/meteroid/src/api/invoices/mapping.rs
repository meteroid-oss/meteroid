pub mod invoices {
    use crate::api::connectors::mapping::connectors::connection_metadata_to_server;
    use crate::api::customers::mapping::customer::ServerAddressWrapper;
    use crate::api::sharable::ShareableEntityClaims;
    use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};
    use common_domain::ids::{BaseId, InvoiceId, TenantId};
    use error_stack::{Report, ResultExt};
    use meteroid_grpc::meteroid::api::invoices::v1::{
        CouponLineItem, DetailedInvoice, InlineCustomer, Invoice, InvoicePaymentStatus,
        InvoiceStatus, InvoiceType, LineItem,
    };
    use meteroid_store::domain::invoice_lines as domain_invoice_lines;
    use meteroid_store::errors::StoreError;
    use meteroid_store::{StoreResult, domain};
    use secrecy::{ExposeSecret, SecretString};

    pub fn status_domain_to_server(value: &domain::enums::InvoiceStatusEnum) -> InvoiceStatus {
        match value {
            domain::enums::InvoiceStatusEnum::Finalized => InvoiceStatus::Finalized,
            domain::enums::InvoiceStatusEnum::Uncollectible => InvoiceStatus::Uncollectible,
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
                    InvoiceStatus::Uncollectible => domain::enums::InvoiceStatusEnum::Uncollectible,
                    InvoiceStatus::Void => domain::enums::InvoiceStatusEnum::Void,
                })
        })
    }

    pub fn payment_status_domain_to_server(
        value: domain::enums::InvoicePaymentStatus,
    ) -> InvoicePaymentStatus {
        match value {
            domain::enums::InvoicePaymentStatus::Paid => InvoicePaymentStatus::Paid,
            domain::enums::InvoicePaymentStatus::PartiallyPaid => {
                InvoicePaymentStatus::PartiallyPaid
            }
            domain::enums::InvoicePaymentStatus::Errored => InvoicePaymentStatus::Errored,
            domain::enums::InvoicePaymentStatus::Unpaid => InvoicePaymentStatus::Unpaid,
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

    pub fn domain_coupon_line_item_to_server(
        line_items: Vec<domain::CouponLineItem>,
    ) -> Vec<CouponLineItem> {
        line_items
            .into_iter()
            .map(|line| CouponLineItem {
                coupon_id: line.coupon_id.as_proto(),
                name: line.name,
                total: line.value,
            })
            .collect()
    }

    pub fn domain_invoice_lines_to_server(line_items: Vec<domain::LineItem>) -> Vec<LineItem> {
        line_items.into_iter()
            .map(|line| {
                LineItem {
                    id: line.local_id,
                    name: line.name,
                    tax_rate: line.tax_rate.as_proto(),
                    metric_id: line.metric_id.map(|x| x.as_proto()),
                    price_component_id: line.price_component_id.map(|x| x.as_proto()),
                    end_date: line.end_date.as_proto(),
                    start_date: line.start_date.as_proto(),
                    quantity: line.quantity.as_proto(),
                    subtotal: line.amount_subtotal,
                    unit_price: line.unit_price.as_proto(),
                    is_prorated: line.is_prorated,
                    product_id: line.product_id.map(|x| x.as_proto()),
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
                                            first_unit,
                                            last_unit,
                                            flat_cap: flat_cap.as_proto(),
                                            flat_fee: flat_fee.as_proto(),
                                        }
                                    ))
                                }
                                Some(domain_invoice_lines::SubLineAttributes::Volume { first_unit, last_unit, flat_cap, flat_fee }) => {
                                    Some(meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::SublineAttributes::Volume(
                                        meteroid_grpc::meteroid::api::invoices::v1::sub_line_item::TieredOrVolume {
                                            first_unit,
                                            last_unit,
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
            .collect()
    }

    pub fn domain_tax_breakdown_to_server(
        item: &domain::TaxBreakdownItem,
    ) -> meteroid_grpc::meteroid::api::invoices::v1::TaxBreakdownItem {
        meteroid_grpc::meteroid::api::invoices::v1::TaxBreakdownItem {
            tax_rate: item.tax_rate.as_proto(),
            name: item.name.clone(),
            amount: item.tax_amount as i64,
        }
    }

    pub fn domain_inline_customer_to_server(
        customer: &domain::InlineCustomer,
    ) -> Result<InlineCustomer, Report<StoreError>> {
        Ok(InlineCustomer {
            id: customer.id.as_proto(),
            name: customer.name.clone(),
            email: customer.email.clone(),
            vat_number: customer.vat_number.clone(),
            snapshot_at: customer.snapshot_at.as_proto(),
            billing_address: customer
                .billing_address
                .clone()
                .map(ServerAddressWrapper::try_from)
                .transpose()?
                .map(|x: ServerAddressWrapper| x.0),
        })
    }

    pub fn generate_invoice_share_key(
        invoice_id: InvoiceId,
        tenant_id: TenantId,
        jwt_secret: &SecretString,
        exp: usize,
    ) -> StoreResult<String> {
        let claims = ShareableEntityClaims {
            exp,
            sub: invoice_id.to_string(),
            entity_id: invoice_id.as_uuid(),
            tenant_id,
        };

        let encoded = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(jwt_secret.expose_secret().as_bytes()),
        )
        .change_context(StoreError::CryptError(
            "Failed to encode shareable claims".to_string(),
        ))?;

        Ok(encoded)
    }

    pub fn domain_invoice_with_transactions_to_server(
        invoice: domain::Invoice,
        transactions: Vec<domain::PaymentTransaction>,
        jwt_secret: SecretString,
    ) -> Result<DetailedInvoice, Report<StoreError>> {
        let share_key = if invoice.pdf_document_id.is_some() || invoice.xml_document_id.is_some() {
            let encoded = generate_invoice_share_key(
                invoice.id,
                invoice.tenant_id,
                &jwt_secret,
                (chrono::Utc::now() + chrono::Duration::days(7)).timestamp() as usize,
            )?;
            Some(encoded)
        } else {
            None
        };

        let line_items = domain_invoice_lines_to_server(invoice.line_items);

        let coupon_line_items = domain_coupon_line_item_to_server(invoice.coupons);

        Ok(DetailedInvoice {
            id: invoice.id.as_proto(),
            status: status_domain_to_server(&invoice.status).into(),
            created_at: invoice.created_at.as_proto(),
            updated_at: invoice.updated_at.as_proto(),
            tenant_id: invoice.tenant_id.as_proto(),
            customer_id: invoice.customer_id.as_proto(),
            subscription_id: invoice.subscription_id.map(|x| x.as_proto()),
            currency: invoice.currency,
            invoice_number: invoice.invoice_number,
            data_updated_at: invoice.data_updated_at.as_proto(),
            invoice_date: invoice.invoice_date.as_proto(),
            plan_version_id: invoice.plan_version_id.map(|x| x.as_proto()),
            invoice_type: invoicing_type_domain_to_server(invoice.invoice_type).into(),
            finalized_at: invoice.finalized_at.as_proto(),
            subtotal: invoice.subtotal,
            subtotal_recurring: invoice.subtotal_recurring,
            tax_amount: invoice.tax_amount,
            discount: invoice.discount,
            total: invoice.total,
            amount_due: invoice.amount_due,
            net_terms: invoice.net_terms,
            reference: invoice.reference,
            memo: invoice.memo,
            local_id: invoice.id.as_proto(), // todo remove me
            due_at: invoice.due_at.as_proto(),
            plan_name: invoice.plan_name,
            customer_details: Some(domain_inline_customer_to_server(&invoice.customer_details)?),
            line_items,
            coupon_line_items,
            applied_credits: invoice.applied_credits,
            document_sharing_key: share_key,
            pdf_document_id: invoice.pdf_document_id.map(|id| id.as_proto()),
            xml_document_id: invoice.xml_document_id.map(|id| id.as_proto()),
            tax_breakdown: invoice
                .tax_breakdown
                .iter()
                .map(domain_tax_breakdown_to_server)
                .collect(),
            connection_metadata: invoice
                .conn_meta
                .as_ref()
                .map(connection_metadata_to_server),
            payment_status: payment_status_domain_to_server(invoice.payment_status).into(),
            paid_at: invoice.paid_at.as_proto(),
            transactions: transactions
                .into_iter()
                .map(super::transactions::domain_to_server)
                .collect(),
            manual: invoice.manual,
            purchase_order: invoice.purchase_order,
            voided_at: invoice.voided_at.as_proto(),
            marked_as_uncollectible_at: invoice.marked_as_uncollectible_at.as_proto(),
        })
    }

    pub fn domain_to_server(value: domain::InvoiceWithCustomer) -> Invoice {
        Invoice {
            id: value.invoice.id.to_string(),
            invoice_number: value.invoice.invoice_number,
            status: status_domain_to_server(&value.invoice.status).into(),
            invoice_date: value.invoice.invoice_date.to_string(),
            customer_id: value.invoice.customer_id.to_string(),
            customer_name: value.customer.name.to_string(),
            subscription_id: value.invoice.subscription_id.map(|x| x.to_string()),
            currency: value.invoice.currency,
            due_at: value.invoice.due_at.as_proto(),
            total: value.invoice.total,
            payment_status: payment_status_domain_to_server(value.invoice.payment_status).into(),
            manual: value.invoice.manual,
        }
    }
}

pub mod transactions {
    use crate::api::shared::conversions::AsProtoOpt;
    use common_utils::integers::ToNonNegativeU64;
    use meteroid_grpc::meteroid::api::invoices::v1::PaymentMethodInfo;
    use meteroid_grpc::meteroid::api::invoices::v1::Transaction;
    use meteroid_grpc::meteroid::api::invoices::v1::payment_method_info::PaymentMethodTypeEnum;
    use meteroid_grpc::meteroid::api::invoices::v1::transaction::{
        PaymentStatusEnum, PaymentTypeEnum,
    };
    use meteroid_store::domain;

    fn status_domain_to_server(value: domain::enums::PaymentStatusEnum) -> PaymentStatusEnum {
        match value {
            domain::enums::PaymentStatusEnum::Ready => PaymentStatusEnum::Ready,
            domain::enums::PaymentStatusEnum::Pending => PaymentStatusEnum::Pending,
            domain::enums::PaymentStatusEnum::Settled => PaymentStatusEnum::Settled,
            domain::enums::PaymentStatusEnum::Cancelled => PaymentStatusEnum::Cancelled,
            domain::enums::PaymentStatusEnum::Failed => PaymentStatusEnum::Failed,
        }
    }

    fn type_domain_to_server(value: domain::enums::PaymentTypeEnum) -> PaymentTypeEnum {
        match value {
            domain::enums::PaymentTypeEnum::Payment => PaymentTypeEnum::Payment,
            domain::enums::PaymentTypeEnum::Refund => PaymentTypeEnum::Refund,
        }
    }

    fn method_type_domain_to_server(
        value: domain::enums::PaymentMethodTypeEnum,
    ) -> PaymentMethodTypeEnum {
        match value {
            domain::enums::PaymentMethodTypeEnum::Card => PaymentMethodTypeEnum::Card,
            domain::enums::PaymentMethodTypeEnum::Other => PaymentMethodTypeEnum::Other,
            domain::enums::PaymentMethodTypeEnum::DirectDebitAch => {
                PaymentMethodTypeEnum::BankTransfer
            }
            domain::enums::PaymentMethodTypeEnum::DirectDebitBacs => {
                PaymentMethodTypeEnum::BankTransfer
            }
            domain::enums::PaymentMethodTypeEnum::DirectDebitSepa => {
                PaymentMethodTypeEnum::BankTransfer
            }
            domain::enums::PaymentMethodTypeEnum::Transfer => PaymentMethodTypeEnum::BankTransfer,
        }
    }

    pub fn domain_to_server(
        value: domain::payment_transactions::PaymentTransaction,
    ) -> Transaction {
        Transaction {
            id: value.id.as_proto(),
            status: status_domain_to_server(value.status).into(),
            payment_type: type_domain_to_server(value.payment_type).into(),
            currency: value.currency,
            payment_method_id: value.payment_method_id.map(|x| x.as_proto()),
            provider_transaction_id: value.provider_transaction_id,
            amount: value.amount.to_non_negative_u64(),
            error: value.error_type,
            invoice_id: value.invoice_id.as_proto(),
            payment_method_info: None,
            processed_at: value.processed_at.as_proto(),
        }
    }

    pub fn domain_with_method_to_server(
        value: domain::payment_transactions::PaymentTransactionWithMethod,
    ) -> Transaction {
        let mut tx = domain_to_server(value.transaction);
        tx.payment_method_info = value.method.map(|m| PaymentMethodInfo {
            account_number_hint: m.account_number_hint,
            card_brand: m.card_brand,
            card_last4: m.card_last4,
            payment_method_type: method_type_domain_to_server(m.payment_method_type).into(),
        });
        tx
    }
}
