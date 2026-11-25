//! The customer resources of API v2 (ADR-0013 §11): what a customer reads as, what creating one
//! takes (and what a merge patch is applied to), and the portal token.
use chrono::{DateTime, Utc};
use common_domain::country::CountryCode;
use common_domain::ids::{CustomerId, InvoicingEntityId};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::enums::CustomerType;
use meteroid_store::domain::{self, CustomerTaxRate, CustomerTaxStatus, VatNumberValidationStatus};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ── Shared ──────────────────────────────────────────────────────────

/// A company, or a person (B2C): a person is named by `first_name` and `last_name`, and needs a
/// billing country.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CustomerKind {
    #[default]
    Company,
    Individual,
}

impl From<CustomerType> for CustomerKind {
    fn from(kind: CustomerType) -> Self {
        match kind {
            CustomerType::Company => CustomerKind::Company,
            CustomerType::Individual => CustomerKind::Individual,
        }
    }
}

impl From<CustomerKind> for CustomerType {
    fn from(kind: CustomerKind) -> Self {
        match kind {
            CustomerKind::Company => CustomerType::Company,
            CustomerKind::Individual => CustomerType::Individual,
        }
    }
}

/// How the customer's invoices are taxed: normally, not at all (with an `exemption_reason`), or
/// with the tax due by the customer (reverse charge).
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaxStatus {
    #[default]
    Taxable,
    Exempt,
    ReverseCharge,
}

impl From<CustomerTaxStatus> for TaxStatus {
    fn from(status: CustomerTaxStatus) -> Self {
        match status {
            CustomerTaxStatus::Taxable => TaxStatus::Taxable,
            CustomerTaxStatus::Exempt => TaxStatus::Exempt,
            CustomerTaxStatus::ReverseCharge => TaxStatus::ReverseCharge,
        }
    }
}

impl From<TaxStatus> for CustomerTaxStatus {
    fn from(status: TaxStatus) -> Self {
        match status {
            TaxStatus::Taxable => CustomerTaxStatus::Taxable,
            TaxStatus::Exempt => CustomerTaxStatus::Exempt,
            TaxStatus::ReverseCharge => CustomerTaxStatus::ReverseCharge,
        }
    }
}

/// The VIES check of the VAT number: pending, valid, invalid, or VIES could not answer.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VatNumberStatus {
    Pending,
    Valid,
    Invalid,
    Unavailable,
}

impl From<VatNumberValidationStatus> for VatNumberStatus {
    fn from(status: VatNumberValidationStatus) -> Self {
        match status {
            VatNumberValidationStatus::Pending => VatNumberStatus::Pending,
            VatNumberValidationStatus::Valid => VatNumberStatus::Valid,
            VatNumberValidationStatus::Invalid => VatNumberStatus::Invalid,
            VatNumberValidationStatus::Unavailable => VatNumberStatus::Unavailable,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Address {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// State, province or region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    /// ISO 3166-1 alpha-2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<CountryCode>,
}

impl From<domain::Address> for Address {
    fn from(address: domain::Address) -> Self {
        Address {
            line1: address.line1,
            line2: address.line2,
            city: address.city,
            state: address.state,
            postal_code: address.zip_code,
            country: address.country,
        }
    }
}

impl From<Address> for domain::Address {
    fn from(address: Address) -> Self {
        domain::Address {
            line1: address.line1,
            line2: address.line2,
            city: address.city,
            state: address.state,
            zip_code: address.postal_code,
            country: address.country,
        }
    }
}

/// Where goods are shipped: the billing address, or another.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ShippingAddress {
    #[serde(default)]
    pub same_as_billing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
}

impl From<domain::ShippingAddress> for ShippingAddress {
    fn from(shipping: domain::ShippingAddress) -> Self {
        ShippingAddress {
            same_as_billing: shipping.same_as_billing,
            address: shipping.address.map(Into::into),
        }
    }
}

impl From<ShippingAddress> for domain::ShippingAddress {
    fn from(shipping: ShippingAddress) -> Self {
        domain::ShippingAddress {
            same_as_billing: shipping.same_as_billing,
            address: shipping.address.map(Into::into),
        }
    }
}

/// A tax rate applied to this customer's invoices in place of the computed ones.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CustomTaxRate {
    pub tax_code: String,
    pub name: String,
    /// A fraction as a decimal string: `"0.2"` is 20%.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub rate: Decimal,
}

impl From<CustomerTaxRate> for CustomTaxRate {
    fn from(rate: CustomerTaxRate) -> Self {
        CustomTaxRate {
            tax_code: rate.tax_code,
            name: rate.name,
            rate: rate.rate,
        }
    }
}

impl From<CustomTaxRate> for CustomerTaxRate {
    fn from(rate: CustomTaxRate) -> Self {
        CustomerTaxRate {
            tax_code: rate.tax_code,
            name: rate.name,
            rate: rate.rate,
        }
    }
}

// ── Reading ─────────────────────────────────────────────────────────

/// How the customer is taxed, and the identifiers its invoices carry.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CustomerTax {
    pub status: TaxStatus,
    /// The legal mention printed on an exempt customer's invoices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exemption_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    /// The VIES check of `vat_number`; absent when there is none to check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_number_status: Option<VatNumberStatus>,
    /// The national register identifier (SIREN/SIRET, HRB…), BT-47 of an e-invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_number: Option<String>,
    pub custom_rates: Vec<CustomTaxRate>,
}

/// How the customer's invoices are issued and sent.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CustomerInvoicing {
    /// The invoicing entity issuing them.
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub entity: InvoicingEntityId,
    /// Where invoices are sent, beside `email`.
    pub emails: Vec<String>,
    /// Document languages, most preferred first (BCP 47); empty follows the invoicing entity.
    pub preferred_locales: Vec<String>,
    /// The reference the customer routes invoices by (a Leitweg-ID), BT-10 of an e-invoice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buyer_reference: Option<String>,
}

/// A customer.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct Customer {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CustomerId,
    /// The integrator's own id, unique per tenant; paths take it as `ext:<alias>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(rename = "type")]
    pub kind: CustomerKind,
    /// A company's name, or a person's full name.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    /// The billing email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// What the customer is billed in (ISO 4217).
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_address: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping_address: Option<ShippingAddress>,
    pub tax: CustomerTax,
    pub invoicing: CustomerInvoicing,
    /// Values of the tenant's customer properties, by key.
    #[schema(value_type = HashMap<String, Value>)]
    pub custom_properties: serde_json::Value,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
    pub created_at: ApiDateTime,
    /// When it last changed.
    pub updated_at: ApiDateTime,
}

/// A customer bag of properties that is not an object reads as none.
fn properties(value: serde_json::Value) -> serde_json::Value {
    if value.is_object() {
        value
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    }
}

impl Customer {
    pub fn new(customer: domain::Customer, updated_at: DateTime<Utc>) -> Self {
        Customer {
            id: customer.id,
            alias: customer.alias,
            kind: customer.customer_type.into(),
            name: customer.name,
            first_name: customer.first_name,
            last_name: customer.last_name,
            email: customer.billing_email,
            phone: customer.phone,
            currency: customer.currency,
            billing_address: customer.billing_address.map(Into::into),
            shipping_address: customer.shipping_address.map(Into::into),
            tax: CustomerTax {
                status: customer.tax_status.into(),
                exemption_reason: customer.exemption_reason,
                vat_number_status: customer
                    .vat_number
                    .as_ref()
                    .and(customer.vat_number_validation_status)
                    .map(Into::into),
                vat_number: customer.vat_number,
                legal_number: customer.legal_number,
                custom_rates: customer.custom_taxes.into_iter().map(Into::into).collect(),
            },
            invoicing: CustomerInvoicing {
                entity: customer.invoicing_entity_id,
                emails: customer.invoicing_emails,
                preferred_locales: customer.preferred_locales,
                buyer_reference: customer.buyer_reference,
            },
            custom_properties: properties(customer.custom_properties),
            archived: customer.archived_at.is_some(),
            archived_at: customer.archived_at.map(Into::into),
            created_at: customer.created_at.into(),
            updated_at: updated_at.into(),
        }
    }
}

crate::v2_list!(CustomerList, Customer);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CustomerListQuery {
    /// Matched anywhere in the name, the alias or the email, ignoring case.
    pub search: Option<String>,
    /// The archived customers instead of the live ones.
    pub archived: Option<bool>,
    /// Only those changed after this moment (RFC 3339).
    #[param(format = DateTime)]
    pub updated_after: Option<String>,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

// ── Writing ─────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct CustomerTaxInput {
    #[serde(default)]
    pub status: TaxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exemption_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legal_number: Option<String>,
    #[serde(default)]
    pub custom_rates: Vec<CustomTaxRate>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct CustomerInvoicingInput {
    /// The invoicing entity, by id; the tenant's default when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    /// BCP 47 tags, most preferred first; one at least must be a language invoices are rendered
    /// in.
    #[serde(default)]
    pub preferred_locales: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buyer_reference: Option<String>,
}

/// A customer as written: the body of `POST /customers`, and what a merge patch of
/// `PATCH /customers/{id_or_alias}` applies to.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[schema(as = CreateCustomer)]
#[serde(deny_unknown_fields)]
pub struct CustomerInput {
    /// The integrator's own id, unique per tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: CustomerKind,
    /// Required for a company; a person's is made of `first_name` and `last_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    /// The billing email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// One of the tenant's currencies (ISO 4217).
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_address: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shipping_address: Option<ShippingAddress>,
    #[serde(default)]
    pub tax: CustomerTaxInput,
    #[serde(default)]
    pub invoicing: CustomerInvoicingInput,
    /// Values of the tenant's customer properties, by key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

impl CustomerInput {
    /// The customer as it would be written back unchanged: what a merge patch applies to.
    pub fn of(customer: &domain::Customer) -> Self {
        CustomerInput {
            alias: customer.alias.clone(),
            kind: customer.customer_type.into(),
            name: Some(customer.name.clone()),
            first_name: customer.first_name.clone(),
            last_name: customer.last_name.clone(),
            email: customer.billing_email.clone(),
            phone: customer.phone.clone(),
            currency: customer.currency.clone(),
            billing_address: customer.billing_address.clone().map(Into::into),
            shipping_address: customer.shipping_address.clone().map(Into::into),
            tax: CustomerTaxInput {
                status: customer.tax_status.into(),
                exemption_reason: customer.exemption_reason.clone(),
                vat_number: customer.vat_number.clone(),
                legal_number: customer.legal_number.clone(),
                custom_rates: customer
                    .custom_taxes
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect(),
            },
            invoicing: CustomerInvoicingInput {
                entity: Some(customer.invoicing_entity_id.to_string()),
                emails: customer.invoicing_emails.clone(),
                preferred_locales: customer.preferred_locales.clone(),
                buyer_reference: customer.buyer_reference.clone(),
            },
            custom_properties: Some(properties(customer.custom_properties.clone())),
        }
    }
}

/// A JSON merge patch of a customer (RFC 7396): a field left out is unchanged, `null` clears it,
/// an object (an address, `tax`, `invoicing`, `custom_properties`) is merged field by field, a
/// list is replaced. Every field of the customer as written may be sent.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateCustomer {
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub alias: Option<Option<String>>,
    #[serde(default, rename = "type")]
    pub kind: Option<CustomerKind>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub first_name: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub last_name: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub email: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub phone: Option<Option<String>>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<Address>)]
    pub billing_address: Option<Option<serde_json::Value>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<ShippingAddress>)]
    pub shipping_address: Option<Option<serde_json::Value>>,
    #[serde(default)]
    #[schema(value_type = Option<CustomerTaxInput>)]
    pub tax: Option<serde_json::Value>,
    #[serde(default)]
    #[schema(value_type = Option<CustomerInvoicingInput>)]
    pub invoicing: Option<serde_json::Value>,
    /// Merged by key: `null` removes a property.
    #[serde(default)]
    #[schema(value_type = Option<HashMap<String, Value>>)]
    pub custom_properties: Option<serde_json::Value>,
}

/// What a portal token may do.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortalScope {
    /// Read billing state: subscriptions, usage, invoices, entitlements.
    Read,
    /// Change plans, pay invoices, manage payment methods. Implies `read`.
    Manage,
}

impl From<PortalScope> for common_domain::auth::PortalScope {
    fn from(scope: PortalScope) -> Self {
        match scope {
            PortalScope::Read => common_domain::auth::PortalScope::Read,
            PortalScope::Manage => common_domain::auth::PortalScope::Manage,
        }
    }
}

#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[schema(as = CreatePortalToken)]
#[serde(deny_unknown_fields)]
pub struct PortalTokenInput {
    /// Lifetime in seconds, 60 to 2592000 (30 days); a day by default.
    #[serde(default)]
    pub expires_in: Option<u32>,
    /// Every scope by default; `["read"]` for a token that only reads, e.g. to gate features in
    /// a browser.
    #[serde(default)]
    pub scopes: Option<Vec<PortalScope>>,
}

/// A token opening the customer portal. A call may return the token of an earlier one with the
/// same scopes, with up to 10% less lifetime: rely on `expires_at`.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct PortalToken {
    pub token: String,
    pub expires_at: ApiDateTime,
    /// The hosted portal, the token included.
    pub url: String,
    /// The base URL of the API a browser calls with the token.
    pub api_url: String,
}
