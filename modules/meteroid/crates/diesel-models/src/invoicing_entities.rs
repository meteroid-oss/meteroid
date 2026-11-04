use crate::bank_accounts::BankAccountRow;
use crate::connectors::ConnectorRow;
use crate::enums::{EInvoicingFormatEnum, TaxResolverEnum};
use common_domain::country::CountryCode;
use common_domain::ids::{
    BankAccountId, ConnectorId, InvoicingEntityId, StoredDocumentId, TaxCategoryId, TenantId,
};
use diesel::{AsChangeset, Identifiable, Insertable, Queryable, Selectable};

#[derive(Debug, Clone, Insertable, Queryable, Identifiable, Selectable)]
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
    pub country: CountryCode,
    pub accounting_currency: String,
    pub tenant_id: TenantId,
    pub card_provider_id: Option<ConnectorId>,
    pub bank_account_id: Option<BankAccountId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub tax_resolver: TaxResolverEnum,
    pub consolidate_recurring_invoices: bool,
    pub portal_theme_mode: Option<String>,
    pub portal_roundness: Option<String>,
    pub require_vies_valid_for_reverse_charge: bool,
    pub invoicing_language: Option<String>,
    pub invoice_footer_translations: serde_json::Value,
    pub default_tax_category_id: Option<TaxCategoryId>,
    pub tax_provider_id: Option<ConnectorId>,
    pub einvoicing_format: Option<EInvoicingFormatEnum>,
    pub legal_number: Option<String>,
    pub credit_note_number_pattern: String,
    /// Customer-facing language (emails, portal); NULL = same as the invoice language.
    pub customer_locale: Option<String>,
    /// Element-level Option<> is forced by diesel's TEXT[] mapping; elements are never NULL.
    pub customer_extra_locales: Vec<Option<String>>,
    /// Brand colour for white backgrounds (emails); NULL = derived from `brand_color`.
    pub brand_color_light: Option<String>,
    /// BT-41 / BT-42 / BT-43 — the seller contact XRechnung requires.
    pub contact_name: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_email: Option<String>,
    pub portal_credit_top_up_enabled: bool,
    pub portal_auto_top_up_enabled: bool,
    pub portal_budgets_enabled: bool,
    pub require_billing_information: bool,
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
    pub country: Option<CountryCode>,
    pub accounting_currency: Option<String>,
    pub tax_resolver: Option<TaxResolverEnum>,
    pub consolidate_recurring_invoices: Option<bool>,
    pub portal_theme_mode: Option<Option<String>>,
    pub portal_roundness: Option<Option<String>>,
    pub require_vies_valid_for_reverse_charge: Option<bool>,
    pub invoicing_language: Option<Option<String>>,
    pub invoice_footer_translations: Option<serde_json::Value>,
    pub default_tax_category_id: Option<Option<TaxCategoryId>>,
    pub einvoicing_format: Option<Option<EInvoicingFormatEnum>>,
    pub legal_number: Option<Option<String>>,
    pub credit_note_number_pattern: Option<String>,
    pub customer_locale: Option<Option<String>>,
    pub customer_extra_locales: Option<Vec<Option<String>>>,
    pub brand_color_light: Option<Option<String>>,
    pub portal_credit_top_up_enabled: Option<bool>,
    pub portal_auto_top_up_enabled: Option<bool>,
    pub portal_budgets_enabled: Option<bool>,
    pub contact_name: Option<Option<String>>,
    pub contact_phone: Option<Option<String>>,
    pub contact_email: Option<Option<String>>,
    pub require_billing_information: Option<bool>,
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
