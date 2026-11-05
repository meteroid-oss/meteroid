use crate::addresses::model::{Address, ShippingAddress};
use crate::currencies::model::Currency;
use common_domain::auth::PortalScope;
use common_domain::ids::{CustomerId, InvoicingEntityId, string_serde, string_serde_opt};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

/// Company vs. individual (B2C). Defaults to `COMPANY`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, ToSchema, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomerType {
    #[default]
    Company,
    Individual,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize, Debug)]
pub struct CustomTaxRate {
    pub tax_code: String,
    pub name: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub rate: rust_decimal::Decimal,
}

#[derive(ToSchema, IntoParams, serde::Serialize, serde::Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct CustomerFilters {
    pub search: Option<String>,
    pub archived: Option<bool>,
}

#[derive(ToSchema, IntoParams, serde::Serialize, serde::Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct CustomerListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    #[serde(flatten)]
    #[validate(nested)]
    pub customer_filters: CustomerFilters,
    /// Sort order. Format: `column.direction`. Allowed columns: `name`, `email`, `alias`, `created_at`. Direction: `asc` or `desc`. Default: `created_at.desc`.
    pub order_by: Option<String>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct Customer {
    #[serde(with = "string_serde")]
    pub id: CustomerId,
    pub name: String,
    #[serde(default)]
    pub customer_type: CustomerType,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub currency: Currency,
    #[serde(with = "string_serde")]
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies). Required by XRechnung.
    pub buyer_reference: Option<String>,
    pub custom_taxes: Vec<CustomTaxRate>,
    pub connected_account_id: Option<String>,
    /// User-defined custom property values, keyed by definition `key`.
    #[schema(value_type = serde_json::Value)]
    pub custom_properties: serde_json::Value,
    /// Preferred document languages, most-preferred first (BCP-47 tags, e.g.
    /// `["fr-FR", "en"]`); overrides the invoicing entity default.
    pub preferred_locales: Vec<String>,
    /// Deprecated: the first entry of `preferred_locales`.
    #[schema(deprecated)]
    pub invoicing_language: Option<String>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate, Debug)]
pub struct CustomerCreateRequest {
    /// Required for `COMPANY`. Ignored for `INDIVIDUAL`: derived from `first_name` + `last_name`.
    #[serde(default)]
    pub name: String,
    /// `INDIVIDUAL` requires `first_name`, `last_name`, and a billing-address country.
    #[serde(default)]
    pub customer_type: CustomerType,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    #[serde(default)]
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: Currency,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    #[serde(default, with = "string_serde_opt")]
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies). Required by XRechnung.
    pub buyer_reference: Option<String>,
    #[serde(default)]
    pub custom_taxes: Vec<CustomTaxRate>,
    pub is_tax_exempt: Option<bool>,
    /// Free-text legal exemption mention surfaced on exempt invoices.
    pub exemption_reason: Option<String>,
    pub connected_account_id: Option<String>,
    /// User-defined custom property values, keyed by definition `key`. Validated against the
    /// tenant's `CUSTOMER` property definitions. Omit to leave unset.
    #[serde(default)]
    #[schema(value_type = Option<serde_json::Value>)]
    pub custom_properties: Option<serde_json::Value>,
    /// Preferred document languages, most-preferred first (BCP-47 tags, e.g.
    /// `["fr-FR", "en"]`); overrides the invoicing entity default. The first one the
    /// renderer has a template for wins, so an unsupported entry alongside a supported
    /// one just falls through; a list of only unsupported ones is rejected.
    #[serde(default)]
    pub preferred_locales: Option<Vec<String>>,
    /// Deprecated: use `preferred_locales`. Applied only when `preferred_locales` is absent.
    #[serde(default)]
    #[schema(deprecated)]
    pub invoicing_language: Option<String>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct CustomerUpdateRequest {
    /// Required for `COMPANY`. Ignored for `INDIVIDUAL`: derived from `first_name` + `last_name`.
    #[serde(default)]
    pub name: String,
    /// `INDIVIDUAL` requires `first_name`, `last_name`, and a billing-address country.
    /// Omit to keep the stored type — a full replace never silently reclassifies a customer.
    #[serde(default)]
    pub customer_type: Option<CustomerType>,
    /// Omit to keep the stored value (a full replace does not blank a person's name).
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: Currency,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    #[serde(with = "string_serde")]
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies). Required by XRechnung.
    pub buyer_reference: Option<String>,
    pub custom_taxes: Vec<CustomTaxRate>,
    pub is_tax_exempt: Option<bool>,
    /// User-defined custom property values (full replace). Omit to leave unchanged.
    #[serde(default)]
    #[schema(value_type = Option<serde_json::Value>)]
    pub custom_properties: Option<serde_json::Value>,
    /// Preferred document languages, most-preferred first (BCP-47 tags, e.g.
    /// `["fr-FR", "en"]`); overrides the invoicing entity default. Omit or send `[]` to
    /// reset to that default (full-replace update).
    #[serde(default)]
    pub preferred_locales: Option<Vec<String>>,
    /// Deprecated: use `preferred_locales`. Applied only when `preferred_locales` is absent.
    #[serde(default)]
    #[schema(deprecated)]
    pub invoicing_language: Option<String>,
    /// Free-text legal exemption mention surfaced on exempt invoices.
    pub exemption_reason: Option<String>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate, Default)]
pub struct CustomerPatchRequest {
    pub name: Option<String>,
    pub customer_type: Option<CustomerType>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Option<Vec<String>>,
    pub phone: Option<String>,
    pub currency: Option<Currency>,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    #[serde(default, with = "string_serde_opt")]
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies). Required by XRechnung.
    pub buyer_reference: Option<String>,
    pub custom_taxes: Option<Vec<CustomTaxRate>>,
    pub is_tax_exempt: Option<bool>,
    /// Partial update of custom property values (merge; send a key with `null` to remove it).
    /// Omit to leave unchanged.
    #[serde(default)]
    #[schema(value_type = Option<serde_json::Value>)]
    pub custom_properties: Option<serde_json::Value>,
    /// Preferred document languages, most-preferred first (BCP-47 tags, e.g.
    /// `["fr-FR", "en"]`); overrides the invoicing entity default. Omit to leave
    /// unchanged, send `[]` to reset to that default.
    #[serde(default)]
    pub preferred_locales: Option<Vec<String>>,
    /// Deprecated: use `preferred_locales`. Applied only when `preferred_locales` is absent.
    #[serde(default)]
    #[schema(deprecated)]
    pub invoicing_language: Option<String>,
    /// Free-text legal exemption mention surfaced on exempt invoices.
    pub exemption_reason: Option<String>,
}

// TODO : allow importing from stripe
// => Allow providing a stripe customer id and load the customer methods from stripe

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomerListResponse {
    pub data: Vec<Customer>,
    pub pagination_meta: PaginationResponse,
}

/// What a customer portal token may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomerPortalScope {
    /// Read the customer's billing data: subscriptions, usage, invoices, entitlements.
    Read,
    /// Change plans, pay invoices and manage payment methods. Implies `read`.
    Manage,
}

impl From<CustomerPortalScope> for PortalScope {
    fn from(scope: CustomerPortalScope) -> Self {
        match scope {
            CustomerPortalScope::Read => PortalScope::Read,
            CustomerPortalScope::Manage => PortalScope::Manage,
        }
    }
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Default)]
pub struct CustomerPortalTokenRequest {
    /// Token lifetime in seconds. Defaults to 86400 (24 hours).
    /// Must be between 60 and 2592000 (30 days).
    pub expires_in_seconds: Option<u32>,
    /// Scopes granted to the token. Defaults to `["read", "manage"]`.
    /// Use `["read"]` for tokens that only read billing state, e.g. to gate features in a browser.
    #[schema(min_items = 1)]
    pub scopes: Option<Vec<CustomerPortalScope>>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomerPortalTokenResponse {
    /// Opaque token for portal access
    pub token: String,
    /// When the token expires (RFC 3339)
    pub expires_at: ApiDateTime,
    /// Base URL of the customer portal
    pub portal_url: String,
    /// Hosted customer portal URL, token included
    pub portal_link: String,
    /// Base URL of the public REST API
    pub api_url: String,
}
