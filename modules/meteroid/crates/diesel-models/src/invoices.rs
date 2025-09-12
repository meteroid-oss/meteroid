use crate::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use chrono::NaiveDate;
use chrono::NaiveDateTime;

use crate::customers::CustomerRow;
use crate::plan_versions::PlanVersionRowOverview;
use common_domain::ids::{
    CustomerId, InvoiceId, InvoicingEntityId, PlanVersionId, StoredDocumentId, SubscriptionId,
    TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRow {
    pub id: InvoiceId,
    pub status: InvoiceStatusEnum,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub line_items: serde_json::Value,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub total: i64,
    pub plan_version_id: Option<PlanVersionId>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub net_terms: i32,
    pub memo: Option<String>,
    pub reference: Option<String>,
    pub invoice_number: String,
    pub tax_amount: i64,
    pub subtotal_recurring: i64,
    pub plan_name: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub customer_details: serde_json::Value,
    pub amount_due: i64,
    pub subtotal: i64,
    pub applied_credits: i64,
    pub seller_details: serde_json::Value,
    pub xml_document_id: Option<StoredDocumentId>,
    pub pdf_document_id: Option<StoredDocumentId>,
    pub conn_meta: Option<serde_json::Value>,
    pub auto_advance: bool,
    pub issued_at: Option<NaiveDateTime>,
    pub payment_status: InvoicePaymentStatus,
    pub paid_at: Option<NaiveDateTime>,
    pub discount: i64,
    pub purchase_order: Option<String>,
    pub coupons: serde_json::Value,
    pub tax_breakdown: serde_json::Value,
    pub manual: bool,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRowLinesPatch {
    pub line_items: serde_json::Value,
    pub amount_due: i64,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub total: i64,
    pub tax_amount: i64,
    pub applied_credits: i64,
    pub tax_breakdown: serde_json::Value,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::invoice)]
pub struct InvoiceRowNew {
    pub id: InvoiceId,
    pub status: InvoiceStatusEnum,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub subscription_id: Option<SubscriptionId>,
    pub currency: String,
    pub invoice_number: String,
    pub line_items: serde_json::Value,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<PlanVersionId>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
    pub auto_advance: bool,
    pub payment_status: InvoicePaymentStatus,
    pub coupons: serde_json::Value,
    pub discount: i64,
    pub purchase_order: Option<String>,
    pub tax_breakdown: serde_json::Value,
    pub manual: bool,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceWithCustomerRow {
    #[diesel(embed)]
    pub invoice: InvoiceRow,
    #[diesel(embed)]
    pub customer: CustomerRow,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceLockRow {
    #[diesel(embed)]
    pub invoice: InvoiceRow,
    #[diesel(select_expression = crate::schema::customer::balance_value_cents)]
    #[diesel(select_expression_type = crate::schema::customer::balance_value_cents)]
    pub customer_balance: i64,
    #[diesel(select_expression = crate::schema::customer::invoicing_entity_id)]
    #[diesel(select_expression_type = crate::schema::customer::invoicing_entity_id)]
    pub customer_invoicing_entity_id: InvoicingEntityId,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DetailedInvoiceRow {
    #[diesel(embed)]
    pub invoice: InvoiceRow,
    #[diesel(embed)]
    pub customer: CustomerRow,
    #[diesel(embed)]
    pub plan: Option<PlanVersionRowOverview>,
}
