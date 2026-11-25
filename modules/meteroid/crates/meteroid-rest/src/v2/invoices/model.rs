//! The invoice resources of API v2 (ADR-0013 §11): what an invoice reads as, what its list is
//! filtered by, and what its actions take.
use crate::v2::documents::{
    DocumentCoupon, DocumentCustomer, DocumentFiles, DocumentLine, DocumentTax, EInvoicingStatus,
};
use crate::v2::refs::{CreditNoteRef, CustomerRef, InvoiceRef, SubscriptionRef};
use chrono::NaiveDate;
use common_domain::ids::{InvoiceId, PaymentTransactionId};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::enums::{
    InvoicePaymentStatus as DomainPaymentStatus, InvoiceStatusEnum, InvoiceType as DomainType,
    PaymentStatusEnum, PaymentTypeEnum, ReversalKind as DomainReversalKind,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ── Reading ─────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    /// Still computed: its lines follow the usage until it is finalized.
    Draft,
    /// Issued.
    Finalized,
    /// Issued, then written off.
    Uncollectible,
    /// Issued, then cancelled.
    Void,
    /// A period that settled with nothing to bill, or a draft deleted: never issued.
    Closed,
}

impl From<InvoiceStatusEnum> for InvoiceStatus {
    fn from(status: InvoiceStatusEnum) -> Self {
        match status {
            InvoiceStatusEnum::Draft => InvoiceStatus::Draft,
            InvoiceStatusEnum::Finalized => InvoiceStatus::Finalized,
            InvoiceStatusEnum::Uncollectible => InvoiceStatus::Uncollectible,
            InvoiceStatusEnum::Void => InvoiceStatus::Void,
            InvoiceStatusEnum::Closed => InvoiceStatus::Closed,
        }
    }
}

impl From<InvoiceStatus> for InvoiceStatusEnum {
    fn from(status: InvoiceStatus) -> Self {
        match status {
            InvoiceStatus::Draft => InvoiceStatusEnum::Draft,
            InvoiceStatus::Finalized => InvoiceStatusEnum::Finalized,
            InvoiceStatus::Uncollectible => InvoiceStatusEnum::Uncollectible,
            InvoiceStatus::Void => InvoiceStatusEnum::Void,
            InvoiceStatus::Closed => InvoiceStatusEnum::Closed,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Unpaid,
    PartiallyPaid,
    Paid,
    /// Its last payment failed.
    Errored,
    /// A payment is accepted by the provider, not settled yet (direct debit).
    Processing,
    /// Below the minimum charge: owed on the customer's next invoice and paid with it.
    Carried,
}

impl From<DomainPaymentStatus> for PaymentStatus {
    fn from(status: DomainPaymentStatus) -> Self {
        match status {
            DomainPaymentStatus::Unpaid => PaymentStatus::Unpaid,
            DomainPaymentStatus::PartiallyPaid => PaymentStatus::PartiallyPaid,
            DomainPaymentStatus::Paid => PaymentStatus::Paid,
            DomainPaymentStatus::Errored => PaymentStatus::Errored,
            DomainPaymentStatus::Processing => PaymentStatus::Processing,
            DomainPaymentStatus::Carried => PaymentStatus::Carried,
        }
    }
}

impl From<PaymentStatus> for DomainPaymentStatus {
    fn from(status: PaymentStatus) -> Self {
        match status {
            PaymentStatus::Unpaid => DomainPaymentStatus::Unpaid,
            PaymentStatus::PartiallyPaid => DomainPaymentStatus::PartiallyPaid,
            PaymentStatus::Paid => DomainPaymentStatus::Paid,
            PaymentStatus::Errored => DomainPaymentStatus::Errored,
            PaymentStatus::Processing => DomainPaymentStatus::Processing,
            PaymentStatus::Carried => DomainPaymentStatus::Carried,
        }
    }
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceType {
    /// A period of a subscription.
    Recurring,
    /// Issued by hand, or for a purchase.
    OneOff,
    /// What a subscription change billed at once (prorated seats, a plan or level change…).
    Adjustment,
    /// Usage billed early, past its threshold.
    UsageThreshold,
    /// What a subscription ended at once consumed after its period's own invoice.
    Final,
}

impl From<DomainType> for InvoiceType {
    fn from(kind: DomainType) -> Self {
        match kind {
            DomainType::Recurring => InvoiceType::Recurring,
            DomainType::OneOff => InvoiceType::OneOff,
            DomainType::Adjustment => InvoiceType::Adjustment,
            DomainType::UsageThreshold => InvoiceType::UsageThreshold,
            DomainType::Final => InvoiceType::Final,
        }
    }
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentType {
    /// Money received for the invoice.
    Payment,
    /// Money given back from a payment.
    Refund,
}

impl From<PaymentTypeEnum> for PaymentType {
    fn from(kind: PaymentTypeEnum) -> Self {
        match kind {
            PaymentTypeEnum::Refund => PaymentType::Refund,
            // A credit purchase is collected as any payment is.
            PaymentTypeEnum::Payment | PaymentTypeEnum::TopUp | PaymentTypeEnum::AutoTopup => {
                PaymentType::Payment
            }
        }
    }
}

#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentState {
    Ready,
    Pending,
    Settled,
    Cancelled,
    Failed,
    /// Settled, then wholly clawed back.
    Refunded,
}

impl From<PaymentStatusEnum> for PaymentState {
    fn from(status: PaymentStatusEnum) -> Self {
        match status {
            PaymentStatusEnum::Ready => PaymentState::Ready,
            PaymentStatusEnum::Pending => PaymentState::Pending,
            PaymentStatusEnum::Settled => PaymentState::Settled,
            PaymentStatusEnum::Cancelled => PaymentState::Cancelled,
            PaymentStatusEnum::Failed => PaymentState::Failed,
            PaymentStatusEnum::Refunded => PaymentState::Refunded,
        }
    }
}

/// Why a payment was clawed back. `chargeback`, `debtor_recall` and `dispute` leave the invoice
/// paid and mark the payment disputed; any other makes the amount due again.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReversalKind {
    Refund,
    Chargeback,
    DebtorRecall,
    InsufficientFunds,
    MandateInvalid,
    Returned,
    Dispute,
    Other,
}

impl From<DomainReversalKind> for ReversalKind {
    fn from(kind: DomainReversalKind) -> Self {
        match kind {
            DomainReversalKind::Refund => ReversalKind::Refund,
            DomainReversalKind::Chargeback => ReversalKind::Chargeback,
            DomainReversalKind::DebtorRecall => ReversalKind::DebtorRecall,
            DomainReversalKind::InsufficientFunds => ReversalKind::InsufficientFunds,
            DomainReversalKind::MandateInvalid => ReversalKind::MandateInvalid,
            DomainReversalKind::Returned => ReversalKind::Returned,
            DomainReversalKind::Dispute => ReversalKind::Dispute,
            DomainReversalKind::Migrated | DomainReversalKind::Other => ReversalKind::Other,
        }
    }
}

impl From<ReversalKind> for DomainReversalKind {
    fn from(kind: ReversalKind) -> Self {
        match kind {
            ReversalKind::Refund => DomainReversalKind::Refund,
            ReversalKind::Chargeback => DomainReversalKind::Chargeback,
            ReversalKind::DebtorRecall => DomainReversalKind::DebtorRecall,
            ReversalKind::InsufficientFunds => DomainReversalKind::InsufficientFunds,
            ReversalKind::MandateInvalid => DomainReversalKind::MandateInvalid,
            ReversalKind::Returned => DomainReversalKind::Returned,
            ReversalKind::Dispute => DomainReversalKind::Dispute,
            ReversalKind::Other => DomainReversalKind::Other,
        }
    }
}

/// A payment of the invoice, or a refund from one.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct InvoicePayment {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: PaymentTransactionId,
    #[serde(rename = "type")]
    pub kind: PaymentType,
    pub status: PaymentState,
    /// In minor units.
    pub amount: i64,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<ApiDateTime>,
    /// The provider's id of it, or the reference a payment recorded by hand was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Given back voluntarily: the invoice stays paid, the refund's credit note reduces it.
    pub amount_refunded: i64,
    /// Clawed back (chargeback, recall, lost dispute).
    pub amount_reversed: i64,
    /// Of `amount_reversed`, what disputes took: the invoice stays paid.
    pub amount_disputed: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reversal_kind: Option<ReversalKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reversal_reason: Option<String>,
    /// A refund's payment.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "common_domain::ids::string_serde_opt::serialize"
    )]
    pub refunds_payment: Option<PaymentTransactionId>,
    /// The credit note a refund was drawn against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_note: Option<CreditNoteRef>,
    /// Why it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// An invoice, its amounts in minor units of its currency. Lists carry what the invoice itself
/// holds; `lines`, `payments` and `credit_notes` are read on a single one.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Invoice {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: InvoiceId,
    pub number: String,
    pub status: InvoiceStatus,
    #[serde(rename = "type")]
    pub kind: InvoiceType,
    pub customer: CustomerRef,
    /// The customer as the invoice prints it.
    pub customer_details: DocumentCustomer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    pub currency: String,
    /// When it is issued; a draft's, the date finalizing it today would stamp.
    pub invoice_date: NaiveDate,
    /// The start of the period it bills, or its own date for an invoice of no period.
    pub period_start: NaiveDate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<NaiveDate>,
    /// Days it is due after it is issued.
    pub net_terms: i32,
    /// Its prices were quoted tax included. Amounts are net all the same: the tax was carved out.
    pub tax_inclusive: bool,
    /// Before discounts, credit and tax.
    pub subtotal: i64,
    /// What coupons and discounts took off, before tax.
    pub discount: i64,
    /// Promotional credit taken off before tax.
    pub promotional_credits: i64,
    pub tax: i64,
    /// `subtotal - discount - promotional_credits + tax`.
    pub total: i64,
    /// Paid credit drawn on it, after tax.
    pub applied_credits: i64,
    /// Debts carried in from earlier invoices, less a debt it carried to a later one.
    pub carried_balance: i64,
    /// `total + carried_balance - applied_credits - amount_paid`.
    pub amount_due: i64,
    /// What payments (and credit notes cancelling its debt) settled of it.
    pub amount_paid: i64,
    pub payment_status: PaymentStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<DocumentLine>>,
    pub taxes: Vec<DocumentTax>,
    pub coupons: Vec<DocumentCoupon>,
    /// Its payments and refunds, by when they were processed (those not processed yet first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payments: Option<Vec<InvoicePayment>>,
    /// The credit notes issued against it: `GET /api/v2/credit-notes/{id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_notes: Option<Vec<CreditNoteRef>>,
    pub files: DocumentFiles,
    /// Whether the structured e-invoice was produced; none when e-invoicing was off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub einvoicing_status: Option<EInvoicingStatus>,
    /// The invoice this one corrects (cancelled by a credit note and reissued as this one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrects: Option<InvoiceRef>,
    /// The invoice reissued in place of this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corrected_by: Option<InvoiceRef>,
    /// The consolidated invoice this draft was merged into, which bills it in its place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consolidated_into: Option<InvoiceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purchase_order: Option<String>,
    #[schema(value_type = HashMap<String, Value>)]
    pub custom_properties: serde_json::Value,
    pub created_at: ApiDateTime,
    /// When it last changed: any edit of a draft (its lines, its customer's details, its annexes),
    /// its issue, its payment, its custom properties. What `updated_after` filters on.
    pub updated_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voided_at: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marked_uncollectible_at: Option<ApiDateTime>,
}

crate::v2_list!(InvoiceList, Invoice);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct InvoiceListQuery {
    /// Only this customer's: its id, or `ext:<alias>`.
    pub customer: Option<String>,
    /// Only this subscription's, by id or external id; with the consolidated invoices its drafts
    /// were merged into.
    pub subscription: Option<String>,
    pub status: Option<InvoiceStatus>,
    pub payment_status: Option<PaymentStatus>,
    /// ISO 4217, like `EUR`.
    pub currency: Option<String>,
    /// Only those finalized after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub issued_after: Option<String>,
    /// Only those whose `updated_at` is after this moment (RFC 3339): created or changed since,
    /// a draft's every edit included.
    #[param(format = DateTime)]
    pub updated_after: Option<String>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

// ── Acting ──────────────────────────────────────────────────────────

/// A JSON merge patch of an invoice: its custom properties at any status; its lines, memo,
/// purchase order and due date while it is a draft.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateInvoice {
    /// Merged into the invoice's: a key sent `null` is removed. Validated against the tenant's
    /// invoice custom property definitions.
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
    /// A draft's lines, all of them (a list replaces): a line kept is named by its `id`, a line
    /// left out is removed, a line without an `id` is added. The draft then finalizes from these
    /// lines as written, no longer priced again from the usage.
    #[serde(default)]
    pub lines: Option<Vec<InvoiceLineInput>>,
    /// A draft's; `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub memo: Option<Option<String>>,
    /// A draft's; `null` clears it.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub purchase_order: Option<Option<String>>,
    /// A draft's; `null`: its net terms from the day it is issued.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>, format = Date)]
    pub due_date: Option<Option<NaiveDate>>,
}

impl UpdateInvoice {
    /// Whether it edits what only a draft lets change.
    pub fn edits_draft(&self) -> bool {
        self.lines.is_some()
            || self.memo.is_some()
            || self.purchase_order.is_some()
            || self.due_date.is_some()
    }
}

/// A line written by hand: `quantity × unit_price`, taxed at `tax_rate`. On a tax-inclusive
/// invoice the price includes the tax.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct InvoiceLineInput {
    /// Editing a draft, the `id` of one of its lines: the line is kept (the charge it bills, its
    /// product), what is sent replacing its value and what is left out keeping it. A line of
    /// sub-lines (tiers, packages) keeps its quantity and price. Left out: a new line, which
    /// needs `name`, `quantity` and `unit_price`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "2")]
    pub quantity: Option<Decimal>,
    /// In the currency's major unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "49.50")]
    pub unit_price: Option<Decimal>,
    /// A fraction, `0.2` for 20%; 0 on a new line when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "decimal", example = "0.2")]
    pub tax_rate: Option<Decimal>,
    /// The days it bills, `end_date` exclusive; the invoice's date on a new line when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_date: Option<NaiveDate>,
}

/// The status a one-off invoice is created in.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NewInvoiceStatus {
    /// Left a draft, editable, issued with `POST …/finalize`.
    #[default]
    Draft,
    /// Issued at once: numbered, dated and sent as a finalized draft is.
    Finalized,
}

/// A one-off invoice, written by hand: billed beside any subscription.
#[derive(Deserialize, Serialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateInvoice {
    /// The customer's id, or `ext:<alias>`.
    pub customer: String,
    /// ISO 4217; the customer's when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// Today when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_date: Option<NaiveDate>,
    /// Its net terms from the day it is issued when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<NaiveDate>,
    /// One at least, each with its `name`, `quantity` and `unit_price` (no `id`).
    pub lines: Vec<InvoiceLineInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_order: Option<String>,
    #[serde(default)]
    pub status: NewInvoiceStatus,
    /// Validated against the tenant's invoice custom property definitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

/// A payment received outside Meteroid (a bank transfer, a check).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RecordPayment {
    /// In minor units of the invoice's currency, at most what is due.
    #[schema(minimum = 1, example = 1250)]
    pub amount: i64,
    /// When it was received, at most a day ahead; now when left out.
    #[serde(default)]
    pub paid_at: Option<ApiDateTime>,
    /// The transfer's reference.
    #[serde(default)]
    pub reference: Option<String>,
}

/// The whole amount due received outside Meteroid.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct MarkPaid {
    /// When it was received, at most a day ahead; now when left out.
    #[serde(default)]
    pub paid_at: Option<ApiDateTime>,
    #[serde(default)]
    pub reference: Option<String>,
}

/// A payment recorded by hand, clawed back.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ReversePayment {
    pub kind: ReversalKind,
    #[serde(default)]
    pub reason: Option<String>,
    /// When it was clawed back, at most a day ahead; now when left out.
    #[serde(default)]
    pub reversed_at: Option<ApiDateTime>,
}
