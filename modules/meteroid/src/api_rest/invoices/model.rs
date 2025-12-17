use crate::api_rest::currencies::model::Currency;
use crate::api_rest::model::{PaginatedRequest, PaginationResponse};
use chrono::{NaiveDate, NaiveDateTime};
use common_domain::country::CountryCode;
use common_domain::ids::{
    AliasOr, CustomerId, CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, SubscriptionId,
};
use common_domain::ids::{string_serde, string_serde_opt};
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
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub memo: Option<String>,
    pub line_items: Vec<InvoiceLineItem>,
    pub paid_at: Option<NaiveDateTime>,
    pub tax_breakdown: Vec<TaxBreakdownItem>,
    pub transactions: Vec<Transaction>,
    pub payment_status: InvoicePaymentStatus,
    pub customer_details: CustomerDetails,
    pub applied_credits: i64,
    pub coupons: Vec<CouponLineItem>,
    pub invoice_type: InvoiceType,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub purchase_order: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub finalized_at: Option<NaiveDateTime>,
    pub voided_at: Option<NaiveDateTime>,
    pub marked_as_uncollectible_at: Option<NaiveDateTime>,
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
    pub amount_total: i64,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    #[schema(value_type = String, format = "decimal")]
    pub tax_rate: rust_decimal::Decimal,
    pub sub_line_items: Vec<SubLineItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum InvoiceStatus {
    Draft,
    Finalized,
    Uncollectible,
    Void,
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
    #[serde(default)]
    pub statuses: Option<Vec<InvoiceStatus>>,
}

#[derive(ToSchema, Serialize, Deserialize)]
pub struct InvoiceListResponse {
    pub data: Vec<Invoice>,
    pub pagination_meta: PaginationResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaxBreakdownItem {
    pub taxable_amount: u64,
    pub tax_amount: u64,
    #[schema(value_type = String, format = "decimal")]
    pub tax_rate: rust_decimal::Decimal,
    pub name: String,
    pub exemption_type: Option<TaxExemptionType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum TaxExemptionType {
    ReverseCharge,
    TaxExempt,
    NotRegistered,
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
    pub processed_at: Option<NaiveDateTime>,
    pub payment_method_info: Option<PaymentMethodInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum PaymentStatusEnum {
    Ready,
    Pending,
    Settled,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
pub enum PaymentMethodTypeEnum {
    Card,
    BankTransfer,
    Wallet,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum InvoicePaymentStatus {
    Unpaid,
    PartiallyPaid,
    Paid,
    Errored,
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
    pub snapshot_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<CountryCode>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CouponLineItem {
    pub coupon_id: String,
    pub name: String,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
