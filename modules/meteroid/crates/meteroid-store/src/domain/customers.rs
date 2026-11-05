use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::{CustomerType, PaymentMethodTypeEnum};
use crate::errors::StoreError;
use crate::errors::StoreErrorReport;
use crate::json_value_serde;
use crate::utils::text::blank_to_none;
use chrono::NaiveDateTime;
use common_domain::country::CountryCode;
use common_domain::ids::{
    AliasOr, BaseId, ConnectedAccountId, ConnectorId, CustomerConnectionId, CustomerId,
    CustomerPaymentMethodId, InvoicingEntityId, PaymentReminderSequenceId, TenantId, UserId,
};
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::customers::{CustomerBriefRow, CustomerRowNew, CustomerRowPatch};
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerTaxRate {
    pub tax_code: String,
    pub name: String,
    pub rate: rust_decimal::Decimal,
}

json_value_serde!(CustomerTaxRate);

/// External (VIES) VAT validation state. See the `customer_vat_validation` migration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, o2o)]
#[map_owned(diesel_models::enums::CustomerVatValidationStatusEnum)]
pub enum VatNumberValidationStatus {
    Pending,
    Valid,
    Invalid,
    Unavailable,
}

/// Party tax status (W6): tri-state replacement for the old `is_tax_exempt` bool.
/// `ReverseCharge` is an explicit merchant choice, additive to the VIES-derived
/// reverse charge the built-in engine already computes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, o2o)]
#[map_owned(diesel_models::enums::CustomerTaxStatusEnum)]
pub enum CustomerTaxStatus {
    #[default]
    Taxable,
    Exempt,
    ReverseCharge,
}

impl From<CustomerTaxStatus> for meteroid_tax::CustomerTaxStatus {
    fn from(val: CustomerTaxStatus) -> Self {
        match val {
            CustomerTaxStatus::Taxable => meteroid_tax::CustomerTaxStatus::Taxable,
            CustomerTaxStatus::Exempt => meteroid_tax::CustomerTaxStatus::Exempt,
            CustomerTaxStatus::ReverseCharge => meteroid_tax::CustomerTaxStatus::ReverseCharge,
        }
    }
}

/// Canonicalizes a preference list on its way to the database. Tags we ship no bundle
/// for are refused at the API boundary (`lang::reject_unsupported_preferences`), so
/// what reaches here only needs its casing and duplicates settled.
pub(crate) fn locales_to_row(raw: Vec<String>) -> Vec<Option<String>> {
    common_domain::locale::normalize_locales(raw)
        .into_iter()
        .map(Some)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, o2o)]
#[try_from_owned(CustomerRow, StoreErrorReport)]
pub struct Customer {
    pub id: CustomerId,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
    pub archived_at: Option<NaiveDateTime>,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub phone: Option<String>,
    pub currency: String,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies).
    pub buyer_reference: Option<String>,
    #[map(~.into_iter().flatten().collect())]
    pub invoicing_emails: Vec<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub conn_meta: Option<ConnectionMeta>,
    #[map(~.into())]
    pub tax_status: CustomerTaxStatus,
    pub exemption_reason: Option<String>,
    #[from(serde_json::from_value(~).unwrap_or_default())]
    pub custom_taxes: Vec<CustomerTaxRate>,
    pub vat_number_format_valid: bool,
    pub connected_account_id: Option<ConnectedAccountId>,
    #[map(~.map(Into::into))]
    pub vat_number_validation_status: Option<VatNumberValidationStatus>,
    pub vat_number_checked_at: Option<NaiveDateTime>,
    /// Evidence returned by VIES with the last definitive answer.
    #[from(~.and_then(|v| serde_json::from_value(v).ok()))]
    pub vat_number_vies_check: Option<meteroid_tax::ViesCheckData>,
    /// User-defined property values, keyed by definition `key`. Validated on write against the
    /// tenant's custom property definitions for `CUSTOMER`.
    pub custom_properties: serde_json::Value,
    /// Preferred languages for this customer's rendered documents, most-preferred first
    /// (BCP-47 tags, e.g. `["fr-FR", "en"]`). Empty inherits from the invoicing entity
    /// (then its country, then `en-US`).
    #[map(~.into_iter().flatten().collect())]
    pub preferred_locales: Vec<String>,
    #[map(~.into())]
    pub customer_type: CustomerType,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    /// Internal user who owns this account; `None` = unassigned.
    pub owner_user_id: Option<UserId>,
    pub payment_reminders_opt_out: bool,
    /// Override sequence for this customer's invoices; `None` = resolve by audience.
    pub payment_reminder_sequence_id: Option<PaymentReminderSequenceId>,
}

impl Customer {
    /// `(first_name, last_name)` for an `Individual`; `None` for a company, which is
    /// identified by `name` alone.
    pub fn person_name(&self) -> Option<(&str, &str)> {
        match (self.customer_type, &self.first_name, &self.last_name) {
            (CustomerType::Individual, Some(first), Some(last)) => Some((first, last)),
            _ => None,
        }
    }

    /// Single-field display name for providers without a first/last split: the person's
    /// full name for an `Individual`, the company name otherwise.
    pub fn display_name(&self) -> String {
        match self.person_name() {
            Some((first, last)) => individual_name(first, last),
            None => self.name.clone(),
        }
    }

    /// What an invoicing entity's `require_billing_information` asks of a customer at checkout.
    pub fn has_complete_billing_information(&self) -> bool {
        is_complete_billing_information(
            self.billing_email.as_deref(),
            self.billing_address.as_ref(),
        )
    }
}

pub fn is_complete_billing_information(email: Option<&str>, address: Option<&Address>) -> bool {
    let is_set = |value: Option<&str>| value.is_some_and(|v| !v.trim().is_empty());
    is_set(email)
        && address.is_some_and(|address| {
            is_set(address.line1.as_deref())
                && is_set(address.city.as_deref())
                && is_set(address.zip_code.as_deref())
                && address.country.is_some()
        })
}

/// A `Customer` plus the derived list metrics (active MRR + lifecycle_state).
#[derive(Clone, Debug)]
pub struct CustomerWithMetrics {
    pub customer: Customer,
    pub mrr_cents: i64,
    pub lifecycle_state: String,
}

impl TryFrom<diesel_models::customers::CustomerWithMetricsRow> for CustomerWithMetrics {
    type Error = StoreErrorReport;

    fn try_from(
        row: diesel_models::customers::CustomerWithMetricsRow,
    ) -> Result<Self, Self::Error> {
        Ok(CustomerWithMetrics {
            customer: row.customer.try_into()?,
            mrr_cents: row.mrr_cents,
            lifecycle_state: row.lifecycle_state,
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(CustomerBriefRow)]
#[owned_into(CustomerBriefRow)]
pub struct CustomerBrief {
    pub id: CustomerId,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerNew {
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    // for seeding
    pub force_created_date: Option<NaiveDateTime>,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies).
    pub buyer_reference: Option<String>,
    pub custom_taxes: Vec<CustomerTaxRate>,
    pub tax_status: CustomerTaxStatus,
    pub exemption_reason: Option<String>,
    pub connected_account_id: Option<ConnectedAccountId>,
    /// Raw provided property values (validated & normalized by the store on insert).
    pub custom_properties: serde_json::Value,
    /// Most-preferred first; canonicalized by the store on insert.
    pub preferred_locales: Vec<String>,
    pub customer_type: CustomerType,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

impl CustomerNew {
    pub fn is_valid_vat_number_format(&self) -> bool {
        match self.vat_number {
            Some(ref vat_number) => {
                meteroid_tax::validation::validate_vat_number_format(vat_number)
            }
            None => false,
        }
    }

    /// B2C quick-win rule: an `Individual` needs first + last name and a billing-address
    /// country (for consumer VAT); its `name` is derived. A `Company` needs a `name`.
    pub fn validate_b2c(&self) -> Result<(), StoreError> {
        validate_company_name(self.customer_type, &self.name)?;
        validate_individual_fields(
            self.customer_type,
            self.first_name.as_deref(),
            self.last_name.as_deref(),
            self.billing_address
                .as_ref()
                .and_then(|a| a.country.as_ref()),
        )
    }
}

/// Stored `name` of an `Individual`: "first last".
pub fn individual_name(first_name: &str, last_name: &str) -> String {
    format!("{} {}", first_name.trim(), last_name.trim())
}

/// The `name` to persist: derived from the person name for an `Individual` (the provided
/// one is ignored, so it can't drift), the provided company name otherwise.
pub(crate) fn resolve_name(
    customer_type: CustomerType,
    name: String,
    first_name: Option<&str>,
    last_name: Option<&str>,
) -> String {
    match (customer_type, first_name, last_name) {
        (CustomerType::Individual, Some(first), Some(last)) => individual_name(first, last),
        _ => name,
    }
}

/// A `Company` is identified by its `name`, so it can't be blank.
pub(crate) fn validate_company_name(
    customer_type: CustomerType,
    name: &str,
) -> Result<(), StoreError> {
    if customer_type == CustomerType::Company && name.trim().is_empty() {
        return Err(StoreError::InvalidArgument(
            "A company customer requires a name".to_string(),
        ));
    }
    Ok(())
}

/// Shared `Individual` rule used by both create and update validation.
pub(crate) fn validate_individual_fields(
    customer_type: CustomerType,
    first_name: Option<&str>,
    last_name: Option<&str>,
    country: Option<&CountryCode>,
) -> Result<(), StoreError> {
    if customer_type != CustomerType::Individual {
        return Ok(());
    }
    if first_name.map(str::trim).unwrap_or("").is_empty()
        || last_name.map(str::trim).unwrap_or("").is_empty()
    {
        return Err(StoreError::InvalidArgument(
            "An individual customer requires both first_name and last_name".to_string(),
        ));
    }
    if country.is_none() {
        return Err(StoreError::InvalidArgument(
            "An individual customer requires a billing-address country".to_string(),
        ));
    }
    Ok(())
}

/// `validate_individual_fields` against the *merged* result of applying `patch` to a
/// customer whose stored values are the `current_*` args. A patch that only flips the
/// type must not leave an `Individual` with no name or no billing country.
pub(crate) fn validate_patch_b2c(
    patch: &CustomerPatch,
    current_type: CustomerType,
    current_name: &str,
    current_first: Option<&str>,
    current_last: Option<&str>,
    current_country: Option<&CountryCode>,
) -> Result<(), StoreError> {
    let customer_type = patch.customer_type.unwrap_or(current_type);
    let first_name = patch.first_name.as_deref().or(current_first);
    let last_name = patch.last_name.as_deref().or(current_last);
    // A patch that replaces the address carries the new country; otherwise keep the stored one.
    let country = match patch.billing_address.as_ref() {
        Some(address) => address.country.as_ref(),
        None => current_country,
    };
    // Only when the patch touches the company identity: unrelated (e.g. system) patches must
    // not start failing on a legacy row with a blank name.
    if patch.name.is_some() || patch.customer_type.is_some() {
        let name = patch.name.as_deref().unwrap_or(current_name);
        validate_company_name(customer_type, name)?;
    }
    validate_individual_fields(customer_type, first_name, last_name, country)
}

/// Keeps an `Individual`'s stored `name` in sync with the merged person name whenever the
/// patch touches identity (type, name or first/last). No-op for companies and for patches
/// that leave identity alone. Run after `validate_patch_b2c`.
pub(crate) fn derive_patch_name(
    patch: &mut CustomerPatch,
    current_type: CustomerType,
    current_first: Option<&str>,
    current_last: Option<&str>,
) {
    let touches_identity = patch.customer_type.is_some()
        || patch.name.is_some()
        || patch.first_name.is_some()
        || patch.last_name.is_some();
    if !touches_identity || patch.customer_type.unwrap_or(current_type) != CustomerType::Individual
    {
        return;
    }
    let first_name = patch.first_name.as_deref().or(current_first);
    let last_name = patch.last_name.as_deref().or(current_last);
    if let (Some(first), Some(last)) = (first_name, last_name) {
        patch.name = Some(individual_name(first, last));
    }
}

/// Result of a lenient batch upsert — valid rows are upserted, invalid rows returned as failures.
#[derive(Debug)]
pub struct CustomerBatchResult {
    pub created: Vec<Customer>,
    /// (index in the original input batch, error message)
    pub failures: Vec<(usize, String)>,
}

#[derive(Clone, Debug)]
pub struct CustomerNewWrapper {
    pub inner: CustomerNew,
    pub tenant_id: TenantId,
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number_format_valid: bool,
}

/// Initial external-validation status for a VAT number: `Pending` only when the
/// number is format-valid and VIES can actually verify it, otherwise unset.
pub(crate) fn initial_vies_status(
    vat_number: Option<&str>,
    format_valid: bool,
) -> Option<diesel_models::enums::CustomerVatValidationStatusEnum> {
    match vat_number {
        Some(vat) if format_valid && meteroid_tax::vies::is_vies_eligible(vat) => {
            Some(diesel_models::enums::CustomerVatValidationStatusEnum::Pending)
        }
        _ => None,
    }
}

impl TryInto<CustomerRowNew> for CustomerNewWrapper {
    type Error = Report<StoreError>;

    fn try_into(self) -> Result<CustomerRowNew, Self::Error> {
        let vat_number_validation_status = initial_vies_status(
            self.inner.vat_number.as_deref(),
            self.vat_number_format_valid,
        );
        Ok(CustomerRowNew {
            id: CustomerId::new(),
            name: resolve_name(
                self.inner.customer_type,
                self.inner.name,
                self.inner.first_name.as_deref(),
                self.inner.last_name.as_deref(),
            ),
            tenant_id: self.tenant_id,
            invoicing_entity_id: self.invoicing_entity_id,
            alias: self.inner.alias,
            billing_email: self.inner.billing_email,
            invoicing_emails: self.inner.invoicing_emails.into_iter().map(Some).collect(),
            legal_number: self.inner.legal_number,
            buyer_reference: blank_to_none(self.inner.buyer_reference),
            phone: self.inner.phone,
            currency: self.inner.currency,
            billing_address: self
                .inner
                .billing_address
                .map(std::convert::TryInto::try_into)
                .transpose()?,
            shipping_address: self
                .inner
                .shipping_address
                .map(std::convert::TryInto::try_into)
                .transpose()?,
            created_at: self.inner.force_created_date,
            current_payment_method_id: None,
            vat_number: self.inner.vat_number,
            custom_taxes: serde_json::to_value(&self.inner.custom_taxes).map_err(|e| {
                StoreError::SerdeError("Failed to serialize custom_taxes".to_string(), e)
            })?,
            tax_status: self.inner.tax_status.into(),
            exemption_reason: self.inner.exemption_reason,
            vat_number_format_valid: self.vat_number_format_valid,
            connected_account_id: self.inner.connected_account_id,
            vat_number_validation_status,
            // Raw here; the repository overwrites this with the validated/normalized bag in-tx.
            custom_properties: self.inner.custom_properties,
            preferred_locales: locales_to_row(self.inner.preferred_locales),
            customer_type: self.inner.customer_type.into(),
            first_name: blank_to_none(self.inner.first_name),
            last_name: blank_to_none(self.inner.last_name),
        })
    }
}

#[derive(Clone, Debug, o2o)]
#[owned_try_into(CustomerRowPatch, StoreErrorReport)]
#[ghosts(vat_number_format_valid: None, vat_number_validation_status: None, vat_number_checked_at: None, vat_number_vies_check: None)]
pub struct CustomerPatch {
    pub id: CustomerId,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    #[map(~.map(|v| v.into_iter().map(|t| Some(t.into())).collect()))]
    pub invoicing_emails: Option<Vec<String>>,
    pub phone: Option<String>,
    pub currency: Option<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<Option<String>>,
    pub legal_number: Option<Option<String>>,
    pub buyer_reference: Option<Option<String>>,
    #[map(~.map(|v| serde_json::to_value(&v)).transpose().map_err(| e | {
    StoreError::SerdeError("Failed to serialize custom_taxes".to_string(), e)
    })?)]
    pub custom_taxes: Option<Vec<CustomerTaxRate>>,
    pub current_payment_method_id: Option<Option<CustomerPaymentMethodId>>,
    #[map(~.map(Into::into))]
    pub tax_status: Option<CustomerTaxStatus>,
    pub exemption_reason: Option<Option<String>>,
    pub connected_account_id: Option<Option<ConnectedAccountId>>,
    /// Raw provided delta (validated & merged by the store in-tx). `None` leaves properties as-is.
    pub custom_properties: Option<serde_json::Value>,
    /// `None` leaves the preferences untouched; an empty list resets them to
    /// "inherit from the invoicing entity".
    #[map(~.map(locales_to_row))]
    pub preferred_locales: Option<Vec<String>>,
    #[map(~.map(Into::into))]
    pub customer_type: Option<CustomerType>,
    /// Set-or-leave (unlike `preferred_locales`' reset semantics): `None` keeps the stored
    /// value; a patch cannot blank a person name back to NULL. Use a full update to clear.
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    // Ownership is intentionally NOT patchable here: owner is mutated only through the
    // validated `assign_customer_owner` choke point. A patch preserves it.
}

impl CustomerPatch {
    pub fn is_valid_vat_number_format(&self) -> Option<bool> {
        match self.vat_number.as_ref() {
            Some(Some(vat_number)) => Some(meteroid_tax::validation::validate_vat_number_format(
                vat_number,
            )),
            Some(None) => Some(false),
            None => None,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<CountryCode>, // TODO mandatory ?
    pub state: Option<String>,
    pub zip_code: Option<String>,
}

json_value_serde!(Address);

impl From<Address> for meteroid_tax::Address {
    fn from(val: Address) -> Self {
        meteroid_tax::Address {
            line1: val.line1,
            city: val.city,
            country: val.country,
            region: val.state,
            postal_code: val.zip_code,
        }
    }
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShippingAddress {
    pub address: Option<Address>,
    pub same_as_billing: bool,
}

json_value_serde!(ShippingAddress);

#[derive(Clone, Debug)]
pub struct CustomerUpdate {
    pub id_or_alias: AliasOr<CustomerId>,
    pub name: String,
    pub alias: Option<String>,
    pub billing_email: Option<String>,
    pub invoicing_emails: Vec<String>,
    pub phone: Option<String>,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: InvoicingEntityId,
    pub vat_number: Option<String>,
    /// BT-47 — the buyer's national register identifier (SIREN/SIRET, HRB).
    pub legal_number: Option<String>,
    /// BT-10 — the reference the buyer routes invoices by (a Leitweg-ID for German
    /// public bodies).
    pub buyer_reference: Option<String>,
    pub custom_taxes: Vec<CustomerTaxRate>,
    pub tax_status: CustomerTaxStatus,
    pub exemption_reason: Option<String>,
    /// Raw provided property values (validated & normalized by the store on update, replace mode).
    pub custom_properties: serde_json::Value,
    /// Replace mode: an empty list resets to "inherit from the invoicing entity".
    pub preferred_locales: Vec<String>,
    pub customer_type: CustomerType,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    // Ownership is intentionally NOT settable here: this replace-update always preserves the
    // current owner. Owner is mutated only through the validated `assign_customer_owner` path.
}

impl CustomerUpdate {
    pub fn is_valid_vat_number_format(&self) -> bool {
        match self.vat_number {
            Some(ref vat_number) => {
                meteroid_tax::validation::validate_vat_number_format(vat_number)
            }
            None => false,
        }
    }

    /// See `CustomerNew::validate_b2c`.
    pub fn validate_b2c(&self) -> Result<(), StoreError> {
        validate_company_name(self.customer_type, &self.name)?;
        validate_individual_fields(
            self.customer_type,
            self.first_name.as_deref(),
            self.last_name.as_deref(),
            self.billing_address
                .as_ref()
                .and_then(|a| a.country.as_ref()),
        )
    }
}

#[derive(Clone, Debug, o2o)]
#[from_owned(CustomerConnectionRow)]
#[owned_into(CustomerConnectionRow)]
pub struct CustomerConnection {
    pub id: CustomerConnectionId,
    pub customer_id: CustomerId,
    pub connector_id: ConnectorId,
    #[into(~.map(|v| v.into_iter().map(|t| Some(t.into())).collect()))]
    #[from(~.map(|v| v.into_iter().flatten().map(|t| t.into()).collect()))]
    pub supported_payment_types: Option<Vec<PaymentMethodTypeEnum>>,
    pub external_customer_id: String,
}

#[cfg(test)]
mod tests {
    use super::{
        Address, CustomerPatch, derive_patch_name, is_complete_billing_information, resolve_name,
        validate_company_name, validate_individual_fields, validate_patch_b2c,
    };
    use crate::domain::enums::CustomerType;
    use common_domain::country::CountryCode;
    use common_domain::ids::{BaseId, CustomerId};

    fn ch() -> CountryCode {
        CountryCode::parse_as_opt("CH").unwrap()
    }

    fn empty_patch() -> CustomerPatch {
        CustomerPatch {
            id: CustomerId::new(),
            name: None,
            alias: None,
            billing_email: None,
            invoicing_emails: None,
            phone: None,
            currency: None,
            billing_address: None,
            shipping_address: None,
            invoicing_entity_id: None,
            vat_number: None,
            custom_taxes: None,
            current_payment_method_id: None,
            tax_status: None,
            exemption_reason: None,
            legal_number: None,
            buyer_reference: None,
            connected_account_id: None,
            custom_properties: None,
            preferred_locales: None,
            customer_type: None,
            first_name: None,
            last_name: None,
        }
    }

    fn addr(country: Option<CountryCode>) -> Address {
        Address {
            line1: None,
            line2: None,
            city: None,
            country,
            state: None,
            zip_code: None,
        }
    }

    fn complete_address() -> Address {
        Address {
            line1: Some("1 Rue de la Paix".to_string()),
            line2: None,
            city: Some("Zurich".to_string()),
            country: Some(ch()),
            state: None,
            zip_code: Some("8001".to_string()),
        }
    }

    #[test]
    fn billing_information_complete_with_email_and_full_address() {
        assert!(is_complete_billing_information(
            Some("billing@acme.com"),
            Some(&complete_address())
        ));
    }

    #[test]
    fn billing_information_requires_a_non_blank_email() {
        assert!(!is_complete_billing_information(
            None,
            Some(&complete_address())
        ));
        assert!(!is_complete_billing_information(
            Some("   "),
            Some(&complete_address())
        ));
    }

    #[test]
    fn billing_information_requires_every_address_part() {
        let email = Some("billing@acme.com");
        assert!(!is_complete_billing_information(email, None));
        let incomplete = [
            Address {
                country: None,
                ..complete_address()
            },
            Address {
                zip_code: None,
                ..complete_address()
            },
            Address {
                line1: Some(" ".to_string()),
                ..complete_address()
            },
            Address {
                city: Some("  ".to_string()),
                ..complete_address()
            },
        ];
        for address in incomplete {
            assert!(!is_complete_billing_information(email, Some(&address)));
        }
    }

    #[test]
    fn company_never_requires_person_fields() {
        assert!(validate_individual_fields(CustomerType::Company, None, None, None).is_ok());
    }

    #[test]
    fn individual_requires_first_last_and_country() {
        assert!(
            validate_individual_fields(CustomerType::Individual, Some("Erika"), None, Some(&ch()))
                .is_err()
        );
        assert!(
            validate_individual_fields(
                CustomerType::Individual,
                Some("Erika"),
                Some("Muster"),
                None
            )
            .is_err()
        );
        assert!(
            validate_individual_fields(
                CustomerType::Individual,
                Some("  "),
                Some("Muster"),
                Some(&ch())
            )
            .is_err()
        );
        assert!(
            validate_individual_fields(
                CustomerType::Individual,
                Some("Erika"),
                Some("Muster"),
                Some(&ch())
            )
            .is_ok()
        );
    }

    #[test]
    fn patch_flip_to_individual_without_identity_is_rejected() {
        let mut patch = empty_patch();
        patch.customer_type = Some(CustomerType::Individual);
        // current is a company with no person fields / no country.
        assert!(
            validate_patch_b2c(&patch, CustomerType::Company, "Acme", None, None, None).is_err()
        );
    }

    #[test]
    fn patch_flip_to_individual_with_identity_in_patch_is_ok() {
        let mut patch = empty_patch();
        patch.customer_type = Some(CustomerType::Individual);
        patch.first_name = Some("Erika".into());
        patch.last_name = Some("Muster".into());
        patch.billing_address = Some(addr(Some(ch())));
        assert!(
            validate_patch_b2c(&patch, CustomerType::Company, "Acme", None, None, None).is_ok()
        );
    }

    #[test]
    fn patch_that_omits_type_preserves_a_valid_individual() {
        // Changing only an unrelated field on a stored Individual must not re-run into a failure.
        let patch = empty_patch();
        assert!(
            validate_patch_b2c(
                &patch,
                CustomerType::Individual,
                "Erika Muster",
                Some("Erika"),
                Some("Muster"),
                Some(&ch()),
            )
            .is_ok()
        );
    }

    #[test]
    fn patch_replacing_address_without_country_rejects_an_individual() {
        // A replacement address is authoritative: it drops the stored country.
        let mut patch = empty_patch();
        patch.billing_address = Some(addr(None));
        assert!(
            validate_patch_b2c(
                &patch,
                CustomerType::Individual,
                "Erika Muster",
                Some("Erika"),
                Some("Muster"),
                Some(&ch()),
            )
            .is_err()
        );
    }

    #[test]
    fn company_requires_a_name() {
        assert!(validate_company_name(CustomerType::Company, "  ").is_err());
        assert!(validate_company_name(CustomerType::Company, "Acme").is_ok());
        assert!(validate_company_name(CustomerType::Individual, "").is_ok());
    }

    #[test]
    fn individual_name_is_derived_and_ignores_the_provided_one() {
        assert_eq!(
            resolve_name(
                CustomerType::Individual,
                "whatever".into(),
                Some(" Erika "),
                Some("Muster")
            ),
            "Erika Muster"
        );
        assert_eq!(
            resolve_name(
                CustomerType::Company,
                "Acme".into(),
                Some("Erika"),
                Some("Muster")
            ),
            "Acme"
        );
    }

    #[test]
    fn patch_blanking_a_company_name_is_rejected() {
        let mut patch = empty_patch();
        patch.name = Some(" ".into());
        assert!(
            validate_patch_b2c(&patch, CustomerType::Company, "Acme", None, None, None).is_err()
        );
    }

    #[test]
    fn unrelated_patch_ignores_a_legacy_blank_company_name() {
        let mut patch = empty_patch();
        patch.phone = Some("+41".into());
        assert!(validate_patch_b2c(&patch, CustomerType::Company, "", None, None, None).is_ok());
    }

    #[test]
    fn patch_renaming_an_individual_rederives_name() {
        let mut patch = empty_patch();
        patch.last_name = Some("Meier".into());
        patch.name = Some("ignored".into());
        derive_patch_name(
            &mut patch,
            CustomerType::Individual,
            Some("Erika"),
            Some("Muster"),
        );
        assert_eq!(patch.name.as_deref(), Some("Erika Meier"));
    }

    #[test]
    fn patch_flipping_to_company_keeps_the_provided_name() {
        let mut patch = empty_patch();
        patch.customer_type = Some(CustomerType::Company);
        patch.name = Some("Acme".into());
        derive_patch_name(
            &mut patch,
            CustomerType::Individual,
            Some("Erika"),
            Some("Muster"),
        );
        assert_eq!(patch.name.as_deref(), Some("Acme"));
    }

    #[test]
    fn unrelated_patch_on_an_individual_does_not_touch_name() {
        let mut patch = empty_patch();
        patch.phone = Some("+41".into());
        derive_patch_name(
            &mut patch,
            CustomerType::Individual,
            Some("Erika"),
            Some("Muster"),
        );
        assert_eq!(patch.name, None);
    }
}
