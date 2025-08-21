use crate::bank_accounts::BankAccountRow;
use crate::connectors::ConnectorRow;
use crate::enums::TaxResolverEnum;
use common_domain::ids::{
    BankAccountId, ConnectorId, InvoicingEntityId, StoredDocumentId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Insertable, Queryable, Identifiable, Selectable)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityRow {
    pub id: InvoicingEntityId,
    pub is_default: bool,
    pub legal_name: String,
    pub invoice_number_pattern: String,
    pub next_invoice_number: i64,
    pub next_credit_note_number: i64,
    pub grace_period_hours: i32,
    pub net_terms: i32,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<StoredDocumentId>,
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
    pub card_provider_id: Option<ConnectorId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub bank_account_id: Option<BankAccountId>,
    pub tax_resolver: TaxResolverEnum,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityRowPatch {
    pub id: InvoicingEntityId,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
    pub invoice_footer_info: Option<String>,
    pub invoice_footer_legal: Option<String>,
    pub logo_attachment_id: Option<Option<StoredDocumentId>>,
    pub brand_color: Option<Option<String>>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub zip_code: Option<String>,
    pub state: Option<String>,
    pub city: Option<String>,
    pub vat_number: Option<String>,
    pub country: Option<String>,
    pub accounting_currency: Option<String>,
    pub tax_resolver: Option<TaxResolverEnum>,
}

#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityRowProvidersPatch {
    pub id: InvoicingEntityId,
    pub card_provider_id: Option<Option<ConnectorId>>,
    pub direct_debit_provider_id: Option<Option<ConnectorId>>,
    pub bank_account_id: Option<Option<BankAccountId>>,
}

#[derive(Debug, Queryable)]
pub struct InvoicingEntityProvidersRow {
    pub entity: InvoicingEntityRow,
    pub card_provider: Option<ConnectorRow>,
    pub direct_debit_provider: Option<ConnectorRow>,
    pub bank_account: Option<BankAccountRow>,
}
#[derive(Debug, AsChangeset)]
#[diesel(table_name = crate::schema::invoicing_entity)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InvoicingEntityTaxPatch {
    pub id: InvoicingEntityId,
    pub tax_resolver: Option<TaxResolverEnum>,
}
