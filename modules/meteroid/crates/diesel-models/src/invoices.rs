use crate::enums::{InvoiceExternalStatusEnum, InvoiceStatusEnum, InvoiceType};
use chrono::NaiveDate;
use chrono::NaiveDateTime;

use crate::customers::CustomerRow;
use crate::plan_versions::PlanVersionRowOverview;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};
use uuid::Uuid;

#[derive(Debug, Identifiable, Queryable, Selectable)]
#[diesel(table_name = crate::schema::invoice)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoiceRow {
    pub id: Uuid,
    pub status: InvoiceStatusEnum,
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Option<Uuid>,
    pub currency: String,
    pub external_invoice_id: Option<String>,
    pub line_items: serde_json::Value,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub total: i64,
    pub plan_version_id: Option<Uuid>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub net_terms: i32,
    pub memo: Option<String>,
    pub tax_rate: i32,
    pub local_id: String,
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
    pub xml_document_id: Option<String>,
    pub pdf_document_id: Option<String>,
    pub applied_coupon_ids: Vec<Option<Uuid>>,
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
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::invoice)]
pub struct InvoiceRowNew {
    pub id: Uuid,
    pub status: InvoiceStatusEnum,
    pub external_status: Option<InvoiceExternalStatusEnum>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Option<Uuid>,
    pub currency: String,
    pub external_invoice_id: Option<String>,
    pub invoice_number: String,
    pub line_items: serde_json::Value,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<NaiveDateTime>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub plan_version_id: Option<Uuid>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
    pub subtotal: i64,
    pub subtotal_recurring: i64,
    pub tax_rate: i32,
    pub tax_amount: i64,
    pub total: i64,
    pub amount_due: i64,
    pub net_terms: i32,
    pub reference: Option<String>,
    pub memo: Option<String>,
    pub local_id: String,
    pub due_at: Option<NaiveDateTime>,
    pub plan_name: Option<String>,
    pub customer_details: serde_json::Value,
    pub seller_details: serde_json::Value,
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
pub struct DetailedInvoiceRow {
    #[diesel(embed)]
    pub invoice: InvoiceRow,
    #[diesel(embed)]
    pub customer: CustomerRow,
    #[diesel(embed)]
    pub plan: Option<PlanVersionRowOverview>,
}
