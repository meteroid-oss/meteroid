use chrono::NaiveDateTime;
use uuid::Uuid;

use crate::enums::CreditNoteStatus;
use common_domain::ids::{CreditNoteId, CustomerId, InvoiceId, TenantId};
use diesel::{Identifiable, Queryable};

#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::credit_note)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CreditNoteRow {
    pub id: CreditNoteId,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub refunded_amount_cents: Option<i64>,
    pub credited_amount_cents: Option<i64>,
    pub currency: String,
    pub finalized_at: NaiveDateTime,
    pub plan_version_id: Option<Uuid>,
    pub invoice_id: InvoiceId,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub status: CreditNoteStatus,
}
