use chrono::NaiveDateTime;
use uuid::Uuid;

use diesel::{Identifiable, Insertable, Queryable, Selectable};


#[derive(Debug, Insertable, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityRow {
    pub id: Uuid,
    pub local_id: String,
    pub is_default: bool,
    pub legal_name: String,
    pub invoice_number_pattern: String,
    pub next_invoice_number: i64,
    pub next_credit_note_number: i64,
    pub grace_period_hours: i32,
    pub net_terms: i32,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<String>,
    pub brand_color: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub zip_code: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    pub tax_id: Option<String>,
    pub country: String,
    pub currency: String,
    pub tenant_id: Uuid,
}