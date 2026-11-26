//! The invoice resources of API v2 (ADR-0013 §12): what an invoice reads as, what its list is
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

/// Where an invoice is in its life: `draft`, still computed, its lines following the usage until
/// it is finalized; `finalized`, issued; `uncollectible`, issued then written off; `void`, issued
/// then cancelled; `closed`, never issued (a period that settled with nothing to bill, or a draft
/// deleted).
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

/// What is paid of an invoice: `unpaid`, `partially_paid`, `paid`; `errored`, its last payment
/// failed; `processing`, a payment is accepted by the provider, not settled yet (direct debit);
/// `carried`, below the minimum charge, owed on the customer's next invoice and paid with it.
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

/// Why an invoice was issued: `recurring`, a period of a subscription; `one_off`, by hand or
/// for a purchase; `adjustment`, what a subscription change billed at once (prorated seats, a
/// plan or level change…); `usage_threshold`, usage billed early, past its threshold; `final`,
/// what a subscription ended at once consumed after its period's own invoice.
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

/// `payment`, money received for the invoice; `refund`, money given back from a payment.
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

/// Where a payment is: `ready`, to be sent to the provider; `pending`, waiting for the provider;
/// `settled`; `cancelled`; `failed`; `refunded`, settled then wholly clawed back (a refund, a
/// chargeback, a lost dispute).
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

/// Why a payment was clawed back: `refund`, given back; `chargeback`, by the card holder's bank;
/// `debtor_recall`, a direct debit recalled by the debtor; `insufficient_funds`;
/// `mandate_invalid`, the direct debit mandate was revoked or invalid; `returned`, the bank
/// returned it; `dispute`, lost in a dispute; `other`. `chargeback`, `debtor_recall` and
/// `dispute` leave the invoice paid and mark the payment disputed; any other makes the amount due
/// again.
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
    /// The payment's id: `POST /invoices/{id}/payments/{payment_id}/reverse` names it.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: PaymentTransactionId,
    /// A payment, or a refund from one.
    #[serde(rename = "type")]
    pub kind: PaymentType,
    /// Where it is.
    pub status: PaymentState,
    /// In minor units.
    pub amount: i64,
    /// ISO 4217, the invoice's.
    pub currency: String,
    /// When the provider processed it, or when a payment recorded by hand was received; absent
    /// while it is not processed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<ApiDateTime>,
    /// The provider's id of it, or the reference a payment recorded by hand was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Given back voluntarily, in minor units: the invoice stays paid, the refund's credit note
    /// reduces it.
    pub amount_refunded: i64,
    /// Clawed back (chargeback, recall, lost dispute), in minor units.
    pub amount_reversed: i64,
    /// Of `amount_reversed`, what disputes took, in minor units: the invoice stays paid.
    pub amount_disputed: i64,
    /// Why it was clawed back; absent on a payment never reversed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reversal_kind: Option<ReversalKind>,
    /// The reason given when it was reversed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reversal_reason: Option<String>,
    /// A refund's payment: the payment it gives money back from.
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
    /// The invoice's id, `inv_…`.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: InvoiceId,
    /// Its number as printed, from the invoicing entity's sequence when it is finalized; `draft`
    /// until then.
    pub number: String,
    /// Where it is in its life.
    pub status: InvoiceStatus,
    /// Why it was issued.
    #[serde(rename = "type")]
    pub kind: InvoiceType,
    /// The customer billed.
    pub customer: CustomerRef,
    /// The customer as the invoice prints it.
    pub customer_details: DocumentCustomer,
    /// The subscription it bills; absent on a one-off invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionRef>,
    /// ISO 4217: every amount of the invoice is in its minor units.
    pub currency: String,
    /// When it is issued; a draft's, the date finalizing it today would stamp.
    pub invoice_date: NaiveDate,
    /// The start of the period it bills, or its own date for an invoice of no period.
    pub period_start: NaiveDate,
    /// When it is due: `invoice_date` plus `net_terms`, unless set by hand; absent on a draft
    /// whose due date is computed when it is issued.
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
    /// The tax, the sum of `taxes`.
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
    /// What is paid of it.
    pub payment_status: PaymentStatus,
    /// When it was paid in full.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_at: Option<ApiDateTime>,
    /// Its lines, as issued; read on a single invoice, absent from lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<DocumentLine>>,
    /// Its tax, one entry per rate.
    pub taxes: Vec<DocumentTax>,
    /// The coupons it applied.
    pub coupons: Vec<DocumentCoupon>,
    /// Its payments and refunds, by when they were processed (those not processed yet first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payments: Option<Vec<InvoicePayment>>,
    /// The credit notes issued against it: `GET /api/v2/credit-notes/{id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_notes: Option<Vec<CreditNoteRef>>,
    /// Which of its files (PDF, e-invoice XML) are ready to download.
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
    /// A note printed on the invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// The payment reference a transfer quotes: the number's letters and digits, set when it is
    /// finalized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The customer's purchase order number, printed on the invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purchase_order: Option<String>,
    /// Values of the tenant's invoice properties, by key.
    #[schema(value_type = HashMap<String, Value>)]
    pub custom_properties: serde_json::Value,
    /// When it was created, as a draft or issued at once.
    pub created_at: ApiDateTime,
    /// When it last changed: any edit of a draft (its lines, its customer's details, its annexes),
    /// its issue, its payment, its custom properties. What `updated_after` filters on.
    pub updated_at: ApiDateTime,
    /// When it was issued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalized_at: Option<ApiDateTime>,
    /// When it was voided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voided_at: Option<ApiDateTime>,
    /// When it was written off.
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
    /// Only those in one of these statuses: repeat the parameter for several
    /// (`?status=draft&status=finalized`).
    #[param(style = Form, explode)]
    pub status: Option<Vec<InvoiceStatus>>,
    /// Only those in one of these payment statuses; repeat the parameter for several.
    #[param(style = Form, explode)]
    pub payment_status: Option<Vec<PaymentStatus>>,
    /// ISO 4217, like `EUR`.
    pub currency: Option<String>,
    /// Only those finalized after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub issued_after: Option<String>,
    /// Only those whose `updated_at` is after this moment (RFC 3339): created or changed since,
    /// a draft's every edit included.
    #[param(format = DateTime)]
    pub updated_after: Option<String>,
    /// Only those created after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub created_after: Option<String>,
    /// Only those created before this moment (RFC 3339).
    #[param(format = DateTime)]
    pub created_before: Option<String>,
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
    /// As printed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Printed under its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A decimal string.
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
    /// The day after the last it bills; `start_date` on a new line when left out.
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
    /// A note printed on the invoice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// The customer's purchase order number, printed on the invoice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_order: Option<String>,
    /// `draft` by default.
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
    /// The transfer's reference.
    #[serde(default)]
    pub reference: Option<String>,
}

/// A payment recorded by hand, clawed back.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ReversePayment {
    /// Why it was clawed back.
    pub kind: ReversalKind,
    /// A free-text reason, kept as the payment's `reversal_reason`.
    #[serde(default)]
    pub reason: Option<String>,
    /// When it was clawed back, at most a day ahead; now when left out.
    #[serde(default)]
    pub reversed_at: Option<ApiDateTime>,
}
