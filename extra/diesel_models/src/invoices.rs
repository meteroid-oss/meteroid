
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use uuid::Uuid;
use chrono::DateTime;
use chrono::offset::Utc;
use diesel::{Identifiable, Queryable};
use diesel::sql_types::Nullable;
use crate::enums::{InvoiceExternalStatusEnum, InvoiceStatusEnum, InvoiceType, InvoicingProviderEnum};


#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::invoice)]
pub struct Invoice {
    pub id: Uuid,
    pub status: InvoiceStatusEnum,
    pub external_status: Option<Nullable<InvoiceExternalStatusEnum>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub subscription_id: Uuid,
    pub currency: String,
    pub days_until_due: Option<i32>,
    pub external_invoice_id: Option<String>,
    pub invoice_id: Option<String>,
    pub invoicing_provider: InvoicingProviderEnum,
    pub line_items: serde_json::Value,
    pub issued: bool,
    pub issue_attempts: i32,
    pub last_issue_attempt_at: Option<DateTime<Utc>>,
    pub last_issue_error: Option<String>,
    pub data_updated_at: Option<NaiveDateTime>,
    pub invoice_date: NaiveDate,
    pub amount_cents: Option<i64>,
    pub plan_version_id: Option<Uuid>,
    pub invoice_type: InvoiceType,
    pub finalized_at: Option<NaiveDateTime>,
}
