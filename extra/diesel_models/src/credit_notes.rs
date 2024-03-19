
use chrono::NaiveDateTime;
use uuid::Uuid;


use diesel::{Identifiable, Queryable};
use crate::enums::CreditNoteStatus;



#[derive(Queryable, Debug, Identifiable)]
#[diesel(table_name = crate::schema::credit_note)]
pub struct CreditNote {
    pub id: Uuid,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub refunded_amount_cents: Option<i64>,
    pub credited_amount_cents: Option<i64>,
    pub currency: String,
    pub finalized_at: NaiveDateTime,
    pub plan_version_id: Option<Uuid>,
    pub invoice_id: Uuid,
    pub tenant_id: Uuid,
    pub customer_id: Uuid,
    pub status: CreditNoteStatus,
}