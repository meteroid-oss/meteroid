use crate::api_rest::currencies;
use crate::api_rest::invoices::model::{
    Address, CouponLineItem, CustomerDetails, Invoice, InvoicePaymentStatus, InvoiceType,
    PaymentStatusEnum, PaymentTypeEnum, SubLineItem, TaxBreakdownItem, TaxExemptionType,
    Transaction,
};
use crate::errors::RestApiError;
use meteroid_store::domain;

pub fn map_status_from_rest(
    s: crate::api_rest::invoices::model::InvoiceStatus,
) -> domain::enums::InvoiceStatusEnum {
    match s {
        crate::api_rest::invoices::model::InvoiceStatus::Draft => {
            domain::enums::InvoiceStatusEnum::Draft
        }
        crate::api_rest::invoices::model::InvoiceStatus::Finalized => {
            domain::enums::InvoiceStatusEnum::Finalized
        }
        crate::api_rest::invoices::model::InvoiceStatus::Uncollectible => {
            domain::enums::InvoiceStatusEnum::Uncollectible
        }
        crate::api_rest::invoices::model::InvoiceStatus::Void => {
            domain::enums::InvoiceStatusEnum::Void
        }
    }
}

pub fn map_status_to_rest(
    s: domain::enums::InvoiceStatusEnum,
) -> crate::api_rest::invoices::model::InvoiceStatus {
    match s {
        domain::enums::InvoiceStatusEnum::Draft => {
            crate::api_rest::invoices::model::InvoiceStatus::Draft
        }
        domain::enums::InvoiceStatusEnum::Finalized => {
            crate::api_rest::invoices::model::InvoiceStatus::Finalized
        }
        domain::enums::InvoiceStatusEnum::Uncollectible => {
            crate::api_rest::invoices::model::InvoiceStatus::Uncollectible
        }
        domain::enums::InvoiceStatusEnum::Void => {
            crate::api_rest::invoices::model::InvoiceStatus::Void
        }
    }
}

pub fn domain_to_rest(
    d: domain::Invoice,
    transactions: Vec<domain::PaymentTransaction>,
) -> Result<Invoice, RestApiError> {
    Ok(Invoice {
        id: d.id,
        currency: currencies::mapping::from_str(d.currency.as_str())?,
        invoice_date: d.invoice_date,
        due_date: d.due_at.map(|dt| dt.date()),
        invoice_number: d.invoice_number,
        status: map_status_to_rest(d.status),
        customer_id: d.customer_id,
        subscription_id: d.subscription_id,
        subtotal: d.subtotal,
        subtotal_recurring: d.subtotal_recurring,
        tax_amount: d.tax_amount,
        total: d.total,
        amount_due: d.amount_due,
        memo: d.memo,
        line_items: d
            .line_items
            .into_iter()
            .map(|li| {
                Ok(crate::api_rest::invoices::model::InvoiceLineItem {
                    name: li.name,
                    description: li.description,
                    quantity: li.quantity,
                    unit_price: li.unit_price,
                    amount_total: li.amount_total,
                    start_date: li.start_date,
                    end_date: li.end_date,
                    tax_rate: li.tax_rate,
                    sub_line_items: li.sub_lines.into_iter().map(map_subline_to_rest).collect(),
                })
            })
            .collect::<Result<Vec<_>, RestApiError>>()?,
        paid_at: d.paid_at,
        tax_breakdown: d
            .tax_breakdown
            .into_iter()
            .map(map_tax_breakdown_to_rest)
            .collect(),
        transactions: transactions
            .into_iter()
            .map(map_transaction_to_rest)
            .collect(),
        payment_status: map_payment_status_to_rest(d.payment_status),
        customer_details: map_customer_details_to_rest(d.customer_details),
        applied_credits: d.applied_credits,
        coupons: d.coupons.into_iter().map(map_coupon_to_rest).collect(),
        invoice_type: map_invoice_type_to_rest(d.invoice_type),
        net_terms: d.net_terms,
        reference: d.reference,
        purchase_order: d.purchase_order,
        created_at: d.created_at,
        updated_at: d.updated_at,
        finalized_at: d.finalized_at,
        voided_at: d.voided_at,
        marked_as_uncollectible_at: d.marked_as_uncollectible_at,
    })
}

fn map_tax_breakdown_to_rest(item: domain::TaxBreakdownItem) -> TaxBreakdownItem {
    TaxBreakdownItem {
        taxable_amount: item.taxable_amount,
        tax_amount: item.tax_amount,
        tax_rate: item.tax_rate,
        name: item.name,
        exemption_type: item.exemption_type.map(map_tax_exemption_type_to_rest),
    }
}

fn map_tax_exemption_type_to_rest(exemption: domain::TaxExemptionType) -> TaxExemptionType {
    match exemption {
        domain::TaxExemptionType::ReverseCharge => TaxExemptionType::ReverseCharge,
        domain::TaxExemptionType::TaxExempt => TaxExemptionType::TaxExempt,
        domain::TaxExemptionType::NotRegistered => TaxExemptionType::NotRegistered,
    }
}

fn map_transaction_to_rest(transaction: domain::PaymentTransaction) -> Transaction {
    Transaction {
        id: transaction.id,
        provider_transaction_id: transaction.provider_transaction_id,
        payment_method_id: transaction.payment_method_id,
        amount: transaction.amount as u64,
        currency: transaction.currency,
        error: transaction.error_type,
        status: map_payment_status_enum_to_rest(transaction.status),
        payment_type: map_payment_type_to_rest(transaction.payment_type),
        processed_at: transaction.processed_at,
        payment_method_info: None, // TODO: Need to join with payment method data
    }
}

fn map_payment_status_enum_to_rest(status: domain::enums::PaymentStatusEnum) -> PaymentStatusEnum {
    match status {
        domain::enums::PaymentStatusEnum::Ready => PaymentStatusEnum::Ready,
        domain::enums::PaymentStatusEnum::Pending => PaymentStatusEnum::Pending,
        domain::enums::PaymentStatusEnum::Settled => PaymentStatusEnum::Settled,
        domain::enums::PaymentStatusEnum::Cancelled => PaymentStatusEnum::Cancelled,
        domain::enums::PaymentStatusEnum::Failed => PaymentStatusEnum::Failed,
    }
}

fn map_payment_type_to_rest(payment_type: domain::enums::PaymentTypeEnum) -> PaymentTypeEnum {
    match payment_type {
        domain::enums::PaymentTypeEnum::Payment => PaymentTypeEnum::Payment,
        domain::enums::PaymentTypeEnum::Refund => PaymentTypeEnum::Refund,
    }
}

fn map_payment_status_to_rest(status: domain::enums::InvoicePaymentStatus) -> InvoicePaymentStatus {
    match status {
        domain::enums::InvoicePaymentStatus::Unpaid => InvoicePaymentStatus::Unpaid,
        domain::enums::InvoicePaymentStatus::PartiallyPaid => InvoicePaymentStatus::PartiallyPaid,
        domain::enums::InvoicePaymentStatus::Paid => InvoicePaymentStatus::Paid,
        domain::enums::InvoicePaymentStatus::Errored => InvoicePaymentStatus::Errored,
    }
}

fn map_customer_details_to_rest(details: domain::InlineCustomer) -> CustomerDetails {
    CustomerDetails {
        id: details.id,
        name: details.name,
        email: details.email,
        alias: details.alias,
        vat_number: details.vat_number,
        billing_address: details.billing_address.map(map_address_to_rest),
        snapshot_at: details.snapshot_at,
    }
}

fn map_address_to_rest(address: domain::Address) -> Address {
    Address {
        line1: address.line1,
        line2: address.line2,
        city: address.city,
        country: address.country,
        state: address.state,
        zip_code: address.zip_code,
    }
}

fn map_coupon_to_rest(coupon: domain::CouponLineItem) -> CouponLineItem {
    CouponLineItem {
        coupon_id: coupon.coupon_id.to_string(),
        name: coupon.name,
        total: coupon.value,
    }
}

fn map_invoice_type_to_rest(invoice_type: domain::enums::InvoiceType) -> InvoiceType {
    match invoice_type {
        domain::enums::InvoiceType::Recurring => InvoiceType::Recurring,
        domain::enums::InvoiceType::OneOff => InvoiceType::OneOff,
        domain::enums::InvoiceType::Adjustment => InvoiceType::Adjustment,
        domain::enums::InvoiceType::UsageThreshold => InvoiceType::UsageThreshold,
    }
}

fn map_subline_to_rest(subline: domain::invoice_lines::SubLineItem) -> SubLineItem {
    SubLineItem {
        id: subline.local_id,
        name: subline.name,
        total: subline.total,
        quantity: subline.quantity,
        unit_price: subline.unit_price,
    }
}
