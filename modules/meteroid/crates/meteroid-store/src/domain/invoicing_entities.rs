use secrecy::SecretString;

use crate::StoreResult;
use crate::domain::connectors::{Connector, ConnectorMeta};
use crate::domain::{Address, BankAccount, TaxResolverEnum};
use crate::utils::text::blank_to_none;
use common_domain::country::CountryCode;
use common_domain::ids::{
    BankAccountId, ConnectorId, InvoicingEntityId, StoredDocumentId, TaxCategoryId, TenantId,
};
use diesel_models::invoicing_entities::{
    InvoicingEntityProvidersRow, InvoicingEntityRow, InvoicingEntityRowPatch,
    InvoicingEntityRowProvidersPatch,
};
use o2o::o2o;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The structured e-invoicing standard an entity issues under.
///
/// Siblings, not levels — see `meteroid_invoicing::einvoicing::EInvoiceFormat`,
/// which this maps onto at render time.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, o2o)]
#[map_owned(diesel_models::enums::EInvoicingFormatEnum)]
pub enum EInvoicingFormat {
    FacturX,
    XRechnung,
    PeppolBis,
}

/// Locale-specific override of the invoice footer text. A blank field falls back
/// to the entity's base (untagged) footer, so a translation never blanks out text
/// set at the default level.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FooterTranslation {
    pub info: Option<String>,
    pub legal: Option<String>,
}

/// Per-locale invoice-footer overrides, keyed by template locale (`"fr-FR"`). The
/// base `invoice_footer_info`/`invoice_footer_legal` remain the default; at render
/// time the document's resolved language selects an override here when present.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FooterTranslations(pub BTreeMap<String, FooterTranslation>);

impl FooterTranslations {
    /// The `(info, legal)` to print for `template_lang`: the locale override when
    /// its field is non-blank, else the provided base default.
    pub fn resolve(
        &self,
        template_lang: &str,
        base_info: Option<&str>,
        base_legal: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        fn non_blank(s: Option<&str>) -> Option<String> {
            s.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        }
        let over = self.0.get(template_lang);
        (
            non_blank(over.and_then(|t| t.info.as_deref())).or_else(|| non_blank(base_info)),
            non_blank(over.and_then(|t| t.legal.as_deref())).or_else(|| non_blank(base_legal)),
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, o2o)]
#[map_owned(InvoicingEntityRow)]
pub struct InvoicingEntity {
    pub id: InvoicingEntityId,
    pub is_default: bool,

    pub legal_name: String,

    pub invoice_number_pattern: String,
    pub credit_note_number_pattern: String,
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

    // immutable
    pub country: CountryCode,
    // immutable
    pub accounting_currency: String,
    pub tenant_id: TenantId,

    pub card_provider_id: Option<ConnectorId>,
    pub direct_debit_provider_id: Option<ConnectorId>,
    pub bank_account_id: Option<BankAccountId>,
    #[map(~.into())]
    pub tax_resolver: TaxResolverEnum,
    /// Opt-in: when true, this entity merges same-day eligible recurring drafts for a
    /// customer into one consolidated invoice. Off by default.
    pub consolidate_recurring_invoices: bool,
    /// Customer-portal theme default ("light" | "dark"). When None the portal
    /// derives it from the tenant's default invoicing entity.
    pub portal_theme_mode: Option<String>,
    /// Customer-portal control roundness ("Sharp" | "Modern" | "Rounded").
    pub portal_roundness: Option<String>,
    /// Opt-in strictness: reverse charge only for VIES-verified VAT numbers.
    pub require_vies_valid_for_reverse_charge: bool,
    /// Default language for this entity's rendered documents; `None` derives it from
    /// the entity country (then falls back to `en-US`). A customer override wins over it.
    pub invoicing_language: Option<String>,
    /// Per-locale overrides of the footer text; the base fields above are the
    /// default. Stored as JSONB; malformed JSON reads as empty.
    #[from(serde_json::from_value(~).unwrap_or_default())]
    #[into(serde_json::to_value(~).unwrap_or_else(| _ | serde_json::json!({})))]
    pub invoice_footer_translations: FooterTranslations,
    /// Best-effort default tax category for lines whose product carries none.
    pub default_tax_category_id: Option<TaxCategoryId>,
    /// External tax provider (a Tax-typed connector). None = built-in resolver.
    pub tax_provider_id: Option<ConnectorId>,
    /// Opt-in: the structured e-invoicing standard this entity issues under.
    /// `None` keeps PDF-only invoicing.
    #[map(~.map(|x| x.into()))]
    pub einvoicing_format: Option<EInvoicingFormat>,
    /// BT-30 — the seller's national register identifier (SIREN/SIRET, HRB).
    /// Distinct from `vat_number`: not every seller has both, and they identify
    /// different things.
    pub legal_number: Option<String>,
    /// Customer-facing language (emails, portal). `None`: each customer's preferred language,
    /// else the invoice language. Resolved through `services::customer_locale`, never the PDF.
    pub customer_locale: Option<String>,
    /// Further locales a customer's preference may pick on customer-facing surfaces.
    #[from(~.into_iter().flatten().collect())]
    #[into(~.into_iter().map(Some).collect())]
    pub customer_extra_locales: Vec<String>,
    /// Brand colour for white backgrounds (emails); `None` derives it from `brand_color`.
    pub brand_color_light: Option<String>,
    /// Customers may buy credit with a saved payment method from the portal.
    pub portal_credit_top_up_enabled: bool,
    /// Customers may configure automatic top-ups from the portal.
    pub portal_auto_top_up_enabled: bool,
    /// Customers may set and remove their own spend budgets from the portal.
    pub portal_budgets_enabled: bool,
    /// BT-41 / BT-42 / BT-43 — who a buyer contacts about an invoice. XRechnung requires
    /// all three; the email is also the seller's electronic address there.
    pub contact_name: Option<String>,
    pub contact_phone: Option<String>,
    pub contact_email: Option<String>,
    /// Checkout completes only for customers with complete billing information.
    pub require_billing_information: bool,
}

#[cfg(test)]
mod footer_translation_tests {
    use super::*;

    #[test]
    fn resolve_prefers_non_blank_override_then_base() {
        let mut by_locale = BTreeMap::new();
        by_locale.insert(
            "fr-FR".to_string(),
            FooterTranslation {
                info: Some("info FR".into()),
                legal: Some("   ".into()), // blank -> falls back to base
            },
        );
        let t = FooterTranslations(by_locale);

        // fr-FR: info overridden; legal blank -> base legal.
        let (info, legal) = t.resolve("fr-FR", Some("base info"), Some("base legal"));
        assert_eq!(info.as_deref(), Some("info FR"));
        assert_eq!(legal.as_deref(), Some("base legal"));

        // en-US: no override at all -> base for both.
        let (info, legal) = t.resolve("en-US", Some("base info"), Some("base legal"));
        assert_eq!(info.as_deref(), Some("base info"));
        assert_eq!(legal.as_deref(), Some("base legal"));

        // Blank base and no override -> None, so the section drops rather than
        // printing an empty heading.
        assert_eq!(t.resolve("en-US", Some("  "), None), (None, None));
    }
}

impl InvoicingEntity {
    pub fn address(&self) -> Address {
        Address {
            line1: self.address_line1.clone(),
            line2: self.address_line2.clone(),
            zip_code: self.zip_code.clone(),
            state: self.state.clone(),
            city: self.city.clone(),
            country: Some(self.country.clone()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InvoicingEntityNew {
    pub country: Option<CountryCode>,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub credit_note_number_pattern: Option<String>,
    pub next_invoice_number: Option<i64>,
    pub next_credit_note_number: Option<i64>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
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
    pub tax_resolver: TaxResolverEnum,
    pub portal_theme_mode: Option<String>,
    pub portal_roundness: Option<String>,
    pub require_vies_valid_for_reverse_charge: Option<bool>,
    pub invoicing_language: Option<String>,
    pub invoice_footer_translations: Option<FooterTranslations>,
    pub customer_locale: Option<String>,
    pub customer_extra_locales: Option<Vec<String>>,
    pub brand_color_light: Option<String>,
    pub portal_credit_top_up_enabled: Option<bool>,
    pub portal_auto_top_up_enabled: Option<bool>,
    pub portal_budgets_enabled: Option<bool>,
    pub require_billing_information: Option<bool>,
}

#[derive(Clone, Debug, o2o, Default)]
#[owned_into(InvoicingEntityRowPatch)]
#[ghosts(accounting_currency: {None})]
pub struct InvoicingEntityPatch {
    pub id: InvoicingEntityId,
    pub legal_name: Option<String>,
    pub invoice_number_pattern: Option<String>,
    pub credit_note_number_pattern: Option<String>,
    pub grace_period_hours: Option<i32>,
    pub net_terms: Option<i32>,
    // Cleared as `""`, not absence — see `brand_color` below. The templates treat
    // blank as absent, so a cleared footer drops its section rather than printing a
    // heading with nothing under it.
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
    #[map(~.map(|x| x.into()))]
    pub tax_resolver: Option<TaxResolverEnum>,
    pub consolidate_recurring_invoices: Option<bool>,
    pub portal_theme_mode: Option<Option<String>>,
    pub portal_roundness: Option<Option<String>>,
    pub require_vies_valid_for_reverse_charge: Option<bool>,
    /// Outer `None` leaves the default untouched; `Some(None)` (or blank) resets it
    /// so the entity country's locale applies again.
    #[map(~.map(blank_to_none))]
    pub invoicing_language: Option<Option<String>>,
    /// `None` leaves translations untouched; `Some(map)` replaces the whole set.
    #[into(~.map(| t | serde_json::to_value(t).unwrap_or_else(| _ | serde_json::json!({}))))]
    pub invoice_footer_translations: Option<FooterTranslations>,
    pub default_tax_category_id: Option<Option<TaxCategoryId>>,
    /// Outer `None` leaves the setting untouched; `Some(None)` turns structured
    /// e-invoicing back off.
    #[into(~.map(| f | f.map(| f | f.into())))]
    pub einvoicing_format: Option<Option<EInvoicingFormat>>,
    #[map(~.map(blank_to_none))]
    pub legal_number: Option<Option<String>>,
    /// Outer `None` leaves it untouched; `Some(None)` (or blank) follows the invoice language again.
    #[map(~.map(blank_to_none))]
    pub customer_locale: Option<Option<String>>,
    /// `None` leaves the set untouched; `Some(list)` replaces it.
    #[into(~.map(| l | l.into_iter().map(Some).collect()))]
    pub customer_extra_locales: Option<Vec<String>>,
    /// Outer `None` leaves it untouched; `Some(None)` (or blank) clears it. Hex-validated on save.
    pub brand_color_light: Option<Option<String>>,
    pub portal_credit_top_up_enabled: Option<bool>,
    pub portal_auto_top_up_enabled: Option<bool>,
    pub portal_budgets_enabled: Option<bool>,
    #[map(~.map(blank_to_none))]
    pub contact_name: Option<Option<String>>,
    #[map(~.map(blank_to_none))]
    pub contact_phone: Option<Option<String>>,
    #[map(~.map(blank_to_none))]
    pub contact_email: Option<Option<String>>,
    pub require_billing_information: Option<bool>,
}

#[derive(Clone, Debug, o2o, Default)]
#[owned_into(InvoicingEntityRowProvidersPatch)]
pub struct InvoicingEntityProvidersPatch {
    pub id: InvoicingEntityId,
    pub card_provider_id: Option<Option<ConnectorId>>,
    pub direct_debit_provider_id: Option<Option<ConnectorId>>,
    pub bank_account_id: Option<Option<BankAccountId>>,
}

#[derive(Clone, Debug)]
pub struct InvoicingEntityProviders {
    pub id: InvoicingEntityId,
    pub bank_account: Option<BankAccount>,
    pub card_provider: Option<ConnectorMeta>,
    pub direct_debit_provider: Option<ConnectorMeta>,
}

impl From<InvoicingEntityProvidersRow> for InvoicingEntityProviders {
    fn from(row: InvoicingEntityProvidersRow) -> Self {
        Self {
            id: row.entity.id,
            bank_account: row.bank_account.map(BankAccount::from),
            card_provider: row.card_provider.map(ConnectorMeta::from),
            direct_debit_provider: row.direct_debit_provider.map(ConnectorMeta::from),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InvoicingEntityProviderSensitive {
    pub id: InvoicingEntityId,
    pub bank_account: Option<BankAccount>,
    pub card_provider: Option<Connector>,
    pub direct_debit_provider: Option<Connector>,
}

impl InvoicingEntityProviderSensitive {
    pub fn from_row(row: InvoicingEntityProvidersRow, key: &SecretString) -> StoreResult<Self> {
        Ok(Self {
            id: row.entity.id,
            bank_account: row.bank_account.map(BankAccount::from),
            card_provider: row
                .card_provider
                .map(|p| Connector::from_row(key, p))
                .transpose()?,
            direct_debit_provider: row
                .direct_debit_provider
                .map(|p| Connector::from_row(key, p))
                .transpose()?,
        })
    }
}
