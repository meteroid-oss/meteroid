use crate::enums::{PaymentStatusEnum, PaymentTypeEnum};
use chrono::NaiveDateTime;
use common_domain::ids::{CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, StoredDocumentId, TenantId};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Queryable, Debug, Identifiable, Selectable, Clone)]
#[diesel(table_name = crate::schema::payment_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentTransactionRow {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub invoice_id: InvoiceId,
    pub provider_transaction_id: Option<String>,
    pub processed_at: Option<NaiveDateTime>,
    pub refunded_at: Option<NaiveDateTime>,
    pub amount: i64,
    pub currency: String,
    // TODO fees ?
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    // enum ?
    pub error_type: Option<String>,
    pub receipt_pdf_id: Option<StoredDocumentId>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = crate::schema::payment_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PaymentTransactionRowNew {
    pub id: PaymentTransactionId,
    pub tenant_id: TenantId,
    pub invoice_id: InvoiceId,
    pub provider_transaction_id: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub payment_method_id: Option<CustomerPaymentMethodId>,
    pub status: PaymentStatusEnum,
    pub payment_type: PaymentTypeEnum,
    pub error_type: Option<String>,
}

#[derive(AsChangeset)]
#[diesel(table_name = crate::schema::payment_transaction)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(primary_key(id))]
pub struct PaymentTransactionRowPatch {
    pub id: PaymentTransactionId,
    pub status: Option<PaymentStatusEnum>,
    pub error_type: Option<Option<String>>,
    pub processed_at: Option<Option<NaiveDateTime>>,
    pub refunded_at: Option<Option<NaiveDateTime>>,
}
