use super::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;

use common_domain::ids::{CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, TenantId};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use o2o::o2o;

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
}

#[derive(Clone, Debug, o2o)]
#[owned_into(PaymentTransactionRowNew)]
pub struct PaymentTransactionNew {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub invoice_id: InvoiceId,
    pub provider_transaction_id: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    #[map(~.into())]
    pub status: PaymentStatusEnum,
    #[map(~.into())]
    pub payment_type: PaymentTypeEnum,
    pub error_type: Option<String>,
}
