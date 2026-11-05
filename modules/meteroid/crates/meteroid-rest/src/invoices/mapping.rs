use crate::currencies;
use crate::invoices::model::{
    Address, CouponLineItem, CustomerDetails, DeclineKind, Invoice, InvoicePaymentStatus,
    InvoiceType, PaymentStatusEnum, PaymentTypeEnum, RefundMode, ReversalKind, SubLineItem,
    TaxBreakdownItem, TaxExemptionType, Transaction,
};
use common_domain::ids::InvoiceId;
use meteroid_app::errors::RestApiError;
use meteroid_store::domain;

pub fn domain_to_rest(
    d: domain::Invoice,
    transactions: Vec<domain::PaymentTransaction>,
    child_invoice_id: Option<InvoiceId>,
    settled_by_credit_notes: bool,
) -> Result<Invoice, RestApiError> {
    Ok(Invoice {
        id: d.id,
        currency: currencies::mapping::from_str(d.currency.as_str())?,
        invoice_date: d.invoice_date,
        due_date: d.due_at.map(|dt| dt.date()),
        // The stable "moment" of the invoice, always present (see the model doc).
        billing_period_start: Some(d.billing_period_start),
        tax_inclusive: d.tax_inclusive,
        invoice_number: d.invoice_number,
        status: d.status.into(),
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
            .map(map_line_item_to_rest)
            .collect(),
        paid_at: d.paid_at.map(Into::into),
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
        einvoicing_status: d.einvoicing_status.map(Into::into),
        customer_details: map_customer_details_to_rest(d.customer_details),
        applied_credits: d.applied_credits,
        promotional_credits: d.promotional_credits,
        coupons: d.coupons.into_iter().map(map_coupon_to_rest).collect(),
        invoice_type: map_invoice_type_to_rest(d.invoice_type),
        parent_invoice_id: d.parent_invoice_id,
        child_invoice_id,
        net_terms: d.net_terms,
        reference: d.reference,
        purchase_order: d.purchase_order,
        created_at: d.created_at.into(),
        updated_at: d.updated_at.map(Into::into),
        finalized_at: d.finalized_at.map(Into::into),
        voided_at: d.voided_at.map(Into::into),
        marked_as_uncollectible_at: d.marked_as_uncollectible_at.map(Into::into),
        payment_disputed_at: d.payment_disputed_at.map(Into::into),
        settled_by_credit_notes,
        custom_properties: d.custom_properties,
    })
}

pub fn map_tax_breakdown_to_rest(item: domain::TaxBreakdownItem) -> TaxBreakdownItem {
    TaxBreakdownItem {
        taxable_amount: item.taxable_amount,
        tax_amount: item.tax_amount,
        tax_rate: item.tax_rate,
        name: item.name,
        exemption_type: item.exemption_type.map(map_tax_exemption_type_to_rest),
        exemption_reason: item.exemption_reason,
        tax_reference: item.tax_reference,
    }
}

fn map_tax_exemption_type_to_rest(exemption: domain::TaxExemptionType) -> TaxExemptionType {
    match exemption {
        domain::TaxExemptionType::ReverseCharge => TaxExemptionType::ReverseCharge,
        domain::TaxExemptionType::TaxExempt => TaxExemptionType::TaxExempt,
        domain::TaxExemptionType::NotRegistered => TaxExemptionType::NotRegistered,
        domain::TaxExemptionType::Export => TaxExemptionType::Export,
        domain::TaxExemptionType::NoVatTerritory => TaxExemptionType::NoVatTerritory,
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
        processed_at: transaction.processed_at.map(Into::into),
        payment_method_info: None, // TODO: Need to join with payment method data
        amount_refunded: transaction.amount_refunded,
        amount_reversed: transaction.amount_reversed,
        amount_disputed: transaction.amount_disputed,
        amount_reversed_excess: transaction.amount_reversed_excess,
        reversal_kind: transaction.reversal_kind.map(map_reversal_kind_to_rest),
        reversal_reason: transaction.reversal_reason,
        parent_transaction_id: transaction.parent_transaction_id,
        credit_note_id: transaction.credit_note_id,
        refund_mode: transaction.refund_mode.map(map_refund_mode_to_rest),
    }
}

pub fn map_reversal_kind_to_rest(kind: domain::enums::ReversalKind) -> ReversalKind {
    match kind {
        domain::enums::ReversalKind::Refund => ReversalKind::Refund,
        domain::enums::ReversalKind::Chargeback => ReversalKind::Chargeback,
        domain::enums::ReversalKind::DebtorRecall => ReversalKind::DebtorRecall,
        domain::enums::ReversalKind::InsufficientFunds => ReversalKind::InsufficientFunds,
        domain::enums::ReversalKind::MandateInvalid => ReversalKind::MandateInvalid,
        domain::enums::ReversalKind::Returned => ReversalKind::Returned,
        domain::enums::ReversalKind::Dispute => ReversalKind::Dispute,
        domain::enums::ReversalKind::Migrated | domain::enums::ReversalKind::Other => {
            ReversalKind::Other
        }
    }
}

pub fn map_decline_kind_to_rest(kind: domain::enums::DeclineKind) -> DeclineKind {
    match kind {
        domain::enums::DeclineKind::InsufficientFunds => DeclineKind::InsufficientFunds,
        domain::enums::DeclineKind::DoNotHonor => DeclineKind::DoNotHonor,
        domain::enums::DeclineKind::CardExpired => DeclineKind::CardExpired,
        domain::enums::DeclineKind::AuthenticationRequired => DeclineKind::AuthenticationRequired,
        domain::enums::DeclineKind::MandateInactive => DeclineKind::MandateInactive,
        domain::enums::DeclineKind::Fraud => DeclineKind::Fraud,
        domain::enums::DeclineKind::ProcessingError => DeclineKind::ProcessingError,
        domain::enums::DeclineKind::Other => DeclineKind::Other,
        domain::enums::DeclineKind::DoNotRetry => DeclineKind::DoNotRetry,
        domain::enums::DeclineKind::Unconfirmed => DeclineKind::Unconfirmed,
    }
}

pub fn map_refund_mode_to_rest(mode: domain::enums::RefundMode) -> RefundMode {
    match mode {
        domain::enums::RefundMode::Online => RefundMode::Online,
        domain::enums::RefundMode::Offline => RefundMode::Offline,
    }
}

pub fn map_payment_status_enum_to_rest(
    status: domain::enums::PaymentStatusEnum,
) -> PaymentStatusEnum {
    match status {
        domain::enums::PaymentStatusEnum::Ready => PaymentStatusEnum::Ready,
        domain::enums::PaymentStatusEnum::Pending => PaymentStatusEnum::Pending,
        domain::enums::PaymentStatusEnum::Settled => PaymentStatusEnum::Settled,
        domain::enums::PaymentStatusEnum::Cancelled => PaymentStatusEnum::Cancelled,
        domain::enums::PaymentStatusEnum::Failed => PaymentStatusEnum::Failed,
        domain::enums::PaymentStatusEnum::Refunded => PaymentStatusEnum::Refunded,
    }
}

pub fn map_payment_type_to_rest(payment_type: domain::enums::PaymentTypeEnum) -> PaymentTypeEnum {
    match payment_type {
        domain::enums::PaymentTypeEnum::Payment => PaymentTypeEnum::Payment,
        domain::enums::PaymentTypeEnum::Refund => PaymentTypeEnum::Refund,
        // The public enum is unchanged: a credit top-up is collected like any payment.
        domain::enums::PaymentTypeEnum::TopUp | domain::enums::PaymentTypeEnum::AutoTopup => {
            PaymentTypeEnum::Payment
        }
    }
}

fn map_payment_status_to_rest(status: domain::enums::InvoicePaymentStatus) -> InvoicePaymentStatus {
    match status {
        domain::enums::InvoicePaymentStatus::Unpaid => InvoicePaymentStatus::Unpaid,
        domain::enums::InvoicePaymentStatus::PartiallyPaid => InvoicePaymentStatus::PartiallyPaid,
        domain::enums::InvoicePaymentStatus::Paid => InvoicePaymentStatus::Paid,
        domain::enums::InvoicePaymentStatus::Errored => InvoicePaymentStatus::Errored,
        domain::enums::InvoicePaymentStatus::Processing => InvoicePaymentStatus::Processing,
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
        snapshot_at: details.snapshot_at.into(),
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
        // The public enum has no final type: it bills the subscription's usage, as a period does.
        domain::enums::InvoiceType::Final => InvoiceType::Recurring,
    }
}

pub fn map_line_item_to_rest(
    li: domain::invoice_lines::LineItem,
) -> crate::invoices::model::InvoiceLineItem {
    crate::invoices::model::InvoiceLineItem {
        name: li.name,
        description: li.description,
        quantity: li.quantity,
        unit_price: li.unit_price,
        quoted_unit_price: li
            .tax_inclusive_quote
            .as_ref()
            .and_then(|quote| quote.unit_price),
        amount_total: li.amount_total,
        start_date: li.start_date,
        end_date: li.end_date,
        tax_rate: li.tax_rate,
        sub_line_items: li.sub_lines.into_iter().map(map_subline_to_rest).collect(),
        credit_unit: li
            .credit_unit
            .map(|credit| crate::invoices::model::InvoiceLineCreditUnit {
                code: credit.code,
                precision: credit.precision,
                rate: credit.rate,
                unit_price: credit.unit_price,
                consumed: credit.consumed,
                covered: credit.covered,
            }),
    }
}

pub fn map_subline_to_rest(subline: domain::invoice_lines::SubLineItem) -> SubLineItem {
    SubLineItem {
        id: subline.local_id,
        name: subline.name,
        total: subline.total,
        quantity: subline.quantity,
        unit_price: subline.unit_price,
    }
}
