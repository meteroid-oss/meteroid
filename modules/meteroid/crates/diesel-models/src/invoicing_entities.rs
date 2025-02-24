use uuid::Uuid;

use crate::bank_accounts::BankAccountRow;
use crate::connectors::ConnectorRow;
use common_domain::ids::TenantId;
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

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
    pub vat_number: Option<String>,
    pub country: String,
    pub accounting_currency: String,
    pub tenant_id: TenantId,
    pub cc_provider_id: Option<Uuid>,
    pub bank_account_id: Option<Uuid>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityRowPatch {
    pub id: Uuid,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<Option<String>>,
    pub brand_color: Option<Option<String>>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub zip_code: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    pub vat_number: Option<String>,
    pub country: Option<String>,
    pub accounting_currency: Option<String>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityRowProvidersPatch {
    pub id: Uuid,
    pub cc_provider_id: Option<Uuid>,
    pub bank_account_id: Option<Uuid>,
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityProvidersRow {
    #[diesel(embed)]
    pub cc_provider: Option<ConnectorRow>,
    #[diesel(embed)]
    pub bank_account: Option<BankAccountRow>,
}
