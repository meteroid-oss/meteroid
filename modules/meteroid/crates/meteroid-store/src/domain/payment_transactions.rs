use super::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;

use crate::domain::CustomerPaymentMethod;
use common_domain::ids::{
    CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, StoredDocumentId, TenantId,
};
use diesel_models::payments::{
    PaymentTransactionRow, PaymentTransactionWithMethodRow,
};
use o2o::o2o;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, o2o)]
#[from_owned(PaymentTransactionRow)]
pub struct PaymentTransaction {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    // technically we could allow a payment intent to be linked to multiple invoices ? (ex: pay multiple overdue at once)
    pub invoice_id: InvoiceId,
    pub provider_transaction_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i64,
    pub currency: String,
    // TODO fees ?
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    #[map(~.into())]
    pub status: PaymentStatusEnum,
    #[map(~.into())]
    pub payment_type: PaymentTypeEnum,
    // enum ?
    pub error_type: Option<String>,
    pub receipt_pdf_id: Option<StoredDocumentId>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentIntent {
    pub external_id: String,
    pub transaction_id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub amount_requested: i64,
    pub amount_received: Option<i64>,
    pub currency: String,
    pub next_action: Option<String>,
    pub status: PaymentStatusEnum,
    pub last_payment_error: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
}

#[derive(Clone, Debug, o2o)]
#[from_owned(PaymentTransactionWithMethodRow)]
pub struct PaymentTransactionWithMethod {
    #[map(~.into())]
    pub transaction: PaymentTransaction,
    #[map(~.map(|m| m.into()))]
    pub method: Option<CustomerPaymentMethod>,
}
