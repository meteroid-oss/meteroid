use crate::currencies::model::Currency;
use chrono::NaiveDate;
use common_domain::ids::{
    AliasOr, CreditNoteId, CustomerId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId,
    SubscriptionId,
};
use common_domain::ids::{string_serde, string_serde_opt};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Invoice {
    #[serde(with = "string_serde")]
    pub id: InvoiceId,
    pub invoice_number: String,
    pub status: InvoiceStatus,
    #[serde(with = "string_serde")]
    pub customer_id: CustomerId,
    #[serde(default, with = "string_serde_opt")]
    pub subscription_id: Option<SubscriptionId>,
    pub currency: Currency,
    pub invoice_date: NaiveDate,
    pub due_date: Option<NaiveDate>,
    /// The period/moment this invoice is about — the subscription period start, or the invoice's
    /// own date for manual/one-off. Stable and always present, distinct from `invoice_date` (the
    /// emission date). Shown as "Invoice date".
    pub billing_period_start: Option<NaiveDate>,
    /// The prices billed were quoted tax-included. Amounts are net regardless: the tax was
    /// carved out of the quoted price, so `total` is that price to the unit.
    pub tax_inclusive: bool,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub memo: Option<String>,
    pub line_items: Vec<InvoiceLineItem>,
    pub paid_at: Option<ApiDateTime>,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub transactions: Vec<Transaction>,
    pub payment_status: InvoicePaymentStatus,
    /// Absent when the invoicing entity had not opted in to e-invoicing.
    pub einvoicing_status: Option<EInvoicingStatus>,
    pub customer_details: CustomerDetails,
    /// Paid credit applied after tax, in minor units of the invoice currency: it settles part of
    /// `total` like a payment, and `amount_due` is net of it.
    pub applied_credits: i64,
    /// Promotional credit applied before tax (credit included with the plan, coupon credit, bonus
    /// credit), in minor units of the invoice currency. Like a discount, it reduces the taxable
    /// amount: `tax_amount` and `total` already exclude it, `subtotal` does not. Unlike
    /// `applied_credits`, which settles part of the `total` after tax.
    #[serde(default)]
    pub promotional_credits: i64,
    pub coupons: Vec<CouponLineItem>,
    pub invoice_type: InvoiceType,
    /// Set on a corrective invoice: the cancelled invoice this one replaces
    /// ("cancels and replaces"). The original is credited in full by a credit note.
    #[serde(default, with = "string_serde_opt")]
    pub parent_invoice_id: Option<InvoiceId>,
    /// Set on a cancelled invoice that has been reissued: the corrective invoice replacing it.
    #[serde(default, with = "string_serde_opt")]
    pub child_invoice_id: Option<InvoiceId>,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub purchase_order: Option<String>,
    pub created_at: ApiDateTime,
    pub updated_at: Option<ApiDateTime>,
    pub finalized_at: Option<ApiDateTime>,
    pub voided_at: Option<ApiDateTime>,
    pub marked_as_uncollectible_at: Option<ApiDateTime>,
    /// When a dispute or chargeback first took money still lost on this invoice's payments. The
    /// invoice stays paid; absent when nothing is disputed.
    pub payment_disputed_at: Option<ApiDateTime>,
    /// True when the invoice is `PAID` without any payment: finalized credit notes cancelled its
    /// whole total. Nothing was collected, so it books no cash.
    pub settled_by_credit_notes: bool,
    /// User-defined custom property values, keyed by definition `key`.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
}

/// Merge update of an invoice's custom property values (send a key with `null` to remove it).
/// Allowed at any status — custom properties stay editable after the invoice is finalized.
#[derive(Debug, Serialize, Deserialize, ToSchema, Validate)]
pub struct InvoiceCustomPropertiesRequest {
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InvoiceLineItem {
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Option<String>, format = "decimal")]
    #[schema(nullable = false)]
    pub quantity: Option<rust_decimal::Decimal>,
    #[schema(value_type = Option<String>, format = "decimal")]
    #[schema(nullable = false)]
    pub unit_price: Option<rust_decimal::Decimal>,
    /// The tax-included unit price the customer was quoted, on a line billed from
    /// tax-inclusive prices. `unit_price` is its net counterpart.
    #[schema(value_type = Option<String>, format = "decimal")]
    #[schema(nullable = false)]
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub quoted_unit_price: Option<rust_decimal::Decimal>,
    pub amount_total: i64,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[schema(value_type = String, format = "decimal")]
    pub tax_rate: rust_decimal::Decimal,
    pub sub_line_items: Vec<SubLineItem>,
    /// Usage priced in a credit unit. The line's money amounts then only carry the credits the
    /// customer's balance in that unit did not cover, billed at `rate`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub credit_unit: Option<InvoiceLineCreditUnit>,
}

/// How a usage line priced in a credit unit was paid: the credits it consumed, those the
/// customer's balance in the unit covered, and the rate the rest was billed at.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InvoiceLineCreditUnit {
    /// The credit unit's code, e.g. `CREDITS`.
    pub code: String,
    /// Credit amounts are integers in 10^-precision credit.
    pub precision: u8,
    /// Invoice-currency major units per credit.
    #[schema(value_type = String, format = "decimal")]
    pub rate: rust_decimal::Decimal,
    /// Credits per usage unit.
    #[schema(value_type = Option<String>, format = "decimal")]
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub unit_price: Option<rust_decimal::Decimal>,
    /// Credits the usage priced.
    pub consumed: i64,
    /// Of which paid by the customer's credit wallet.
    pub covered: i64,
}

#[derive(o2o, Debug, Clone, Serialize, Deserialize, ToSchema)]
#[map_owned(meteroid_store::domain::enums::InvoiceStatusEnum)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "InvoiceStatus")]
pub enum InvoiceStatus {
    Draft,
    Finalized,
    Uncollectible,
    Void,
    Closed,
}

/// Whether the structured e-invoice was produced with the accounting PDF. Absent when the
/// invoicing entity had not opted in at the time the invoice was issued.
#[derive(o2o, Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[map_owned(meteroid_store::domain::einvoicing::EInvoicingStatus)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(title = "EInvoicingStatus")]
pub enum EInvoicingStatus {
    Generated,
    Failed,
}

#[derive(Deserialize, Validate, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct InvoiceListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Filter by customer ID or alias
    #[param(value_type = String, required = false)]
    pub customer_id: Option<AliasOr<CustomerId>>,
    #[serde(default, with = "string_serde_opt")]
    pub subscription_id: Option<SubscriptionId>,
    #[param(inline)]
    #[serde(default)]
    pub statuses: Option<Vec<InvoiceStatus>>,
    /// Only invoices whose e-invoice was generated, or failed. Invoices from entities that
    /// had not opted in carry no status and match neither.
    #[param(inline)]
    #[serde(default)]
    pub einvoicing_status: Option<EInvoicingStatus>,
    /// Sort order. Format: `column.direction`. Allowed columns: `invoice_number`, `customer_name`, `amount`, `invoice_date`, `status`, `payment_status`. Direction: `asc` or `desc`. Default: `invoice_date.desc`.
    pub order_by: Option<String>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct InvoiceListResponse {
    pub data: Vec<Invoice>,
    pub pagination_meta: PaginationResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaxBreakdownItem {
    pub taxable_amount: i64,
    pub tax_amount: i64,
    #[schema(value_type = String, format = "decimal")]
    pub tax_rate: rust_decimal::Decimal,
    pub name: String,
    pub exemption_type: Option<TaxExemptionType>,
    /// Free-text legal exemption mention (EU exempt/reverse-charge invoices).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exemption_reason: Option<String>,
    /// Accounting/reporting code of the tax rate for this line, for exports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    /// Renamed from `NotRegistered` internally, but the wire value is deliberately
    /// unchanged: integrators match on it, and historical invoices are re-serialised
    /// through this enum on every read, so a rename would silently rewrite the past.
    #[serde(rename = "NOT_REGISTERED")]
    NotRegistered,
    Export,
    /// Taxable in a part of the seller's own VAT territory where VAT does not apply
    /// (French Guiana, Mayotte: CGI art. 294-1). Out of scope like `EXPORT`, but the
    /// supply never left the territory.
    NoVatTerritory,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Transaction {
    #[serde(with = "string_serde")]
    pub id: PaymentTransactionId,
    pub provider_transaction_id: Option<String>,
    #[serde(default, with = "string_serde_opt")]
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub amount: u64,
    pub currency: String,
    pub error: Option<String>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    pub processed_at: Option<ApiDateTime>,
    pub payment_method_info: Option<PaymentMethodInfo>,
    /// On a PAYMENT: how much was voluntarily given back (the sum of its settled REFUND
    /// children). The invoice stays paid — the Refund credit note is what reduces it.
    pub amount_refunded: i64,
    /// On a PAYMENT: how much was involuntarily clawed back (chargeback, bank recall, lost
    /// dispute). This is what the invoice nets out, reopening it.
    pub amount_reversed: i64,
    /// On a PAYMENT: the part of `amount_reversed` lost to disputes or chargebacks. The invoice
    /// stays paid; only the rest of `amount_reversed` (a failed collection) reopens it.
    #[serde(default)]
    pub amount_disputed: i64,
    /// On a PAYMENT: what the provider withdrew above what the payment still held (refunded
    /// before). Never applied to the invoice or credit: contest it with the provider.
    #[serde(default)]
    pub amount_reversed_excess: i64,
    pub reversal_kind: Option<ReversalKind>,
    /// The provider's raw cause, or what the operator typed when reversing by hand.
    pub reversal_reason: Option<String>,
    /// Set on a REFUND: the settled payment the money is given back from.
    #[serde(default, with = "string_serde_opt")]
    pub parent_transaction_id: Option<PaymentTransactionId>,
    /// Set on a REFUND drawn against a finalized Refund credit note.
    #[serde(default, with = "string_serde_opt")]
    pub credit_note_id: Option<CreditNoteId>,
    pub refund_mode: Option<RefundMode>,
}

/// Why a payment was involuntarily clawed back.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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

/// Why the payment provider declined a charge.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeclineKind {
    InsufficientFunds,
    DoNotHonor,
    CardExpired,
    AuthenticationRequired,
    MandateInactive,
    Fraud,
    ProcessingError,
    Other,
    DoNotRetry,
    Unconfirmed,
}

/// How a voluntary refund was issued: through the provider, or recorded after a wire or cash
/// movement made outside Meteroid.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RefundMode {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentStatusEnum {
    Ready,
    Pending,
    Settled,
    Cancelled,
    Failed,
    Refunded,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentTypeEnum {
    Payment,
    Refund,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaymentMethodInfo {
    pub payment_method_type: PaymentMethodTypeEnum,
    pub card_brand: Option<String>,
    pub card_last4: Option<String>,
    pub account_number_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentMethodTypeEnum {
    Card,
    BankTransfer,
    Wallet,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InvoicePaymentStatus {
    Unpaid,
    PartiallyPaid,
    Paid,
    Errored,
    Processing,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CustomerDetails {
    #[serde(with = "string_serde")]
    pub id: CustomerId,
    pub name: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub vat_number: Option<String>,
    pub billing_address: Option<Address>,
    pub snapshot_at: ApiDateTime,
}

pub use crate::addresses::model::Address;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CouponLineItem {
    pub coupon_id: String,
    pub name: String,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InvoiceType {
    Recurring,
    OneOff,
    Adjustment,
    UsageThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubLineItem {
    pub id: String,
    pub name: String,
    pub total: i64,
    #[schema(value_type = String, format = "decimal")]
    pub quantity: rust_decimal::Decimal,
    #[schema(value_type = String, format = "decimal")]
    pub unit_price: rust_decimal::Decimal,
}

/// Unused dummy struct only used for an OpenAPI definition.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct BinaryFile(PhantomData<Vec<u8>>);
