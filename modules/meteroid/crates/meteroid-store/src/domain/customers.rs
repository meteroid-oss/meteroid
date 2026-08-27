use crate::domain::connectors::ConnectionMeta;
use crate::domain::enums::PaymentMethodTypeEnum;
use crate::errors::StoreError;
use crate::errors::StoreErrorReport;
use crate::json_value_serde;
use chrono::NaiveDateTime;
use common_domain::country::CountryCode;
use common_domain::ids::{
    AliasOr, BaseId, ConnectedAccountId, ConnectorId, CustomerConnectionId, CustomerId,
    CustomerPaymentMethodId, InvoicingEntityId, TenantId,
};
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::customers::{CustomerBriefRow, CustomerRowNew, CustomerRowPatch};
use error_stack::Report;
use o2o::o2o;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use uuid::Uuid;

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
    pub balance_value_cents: i64,
    pub currency: String,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub current_payment_method_id: Option<CustomerPaymentMethodId>,
    pub vat_number: Option<String>,
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
}

impl Customer {
    pub fn has_complete_billing_information(&self) -> bool {
        let has_email = self
            .billing_email
            .as_ref()
            .is_some_and(|email| !email.trim().is_empty());

        let has_address = self.billing_address.as_ref().is_some_and(|address| {
            let is_set =
                |value: &Option<String>| value.as_ref().is_some_and(|v| !v.trim().is_empty());
            is_set(&address.line1)
                && is_set(&address.city)
                && is_set(&address.zip_code)
                && address.country.is_some()
        });

        has_email && has_address
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
    pub balance_value_cents: i64,
    pub currency: String,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    // for seeding
    pub force_created_date: Option<NaiveDateTime>,
    pub vat_number: Option<String>,
    pub custom_taxes: Vec<CustomerTaxRate>,
    pub tax_status: CustomerTaxStatus,
    pub exemption_reason: Option<String>,
    pub connected_account_id: Option<ConnectedAccountId>,
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
            name: self.inner.name,
            tenant_id: self.tenant_id,
            invoicing_entity_id: self.invoicing_entity_id,
            alias: self.inner.alias,
            billing_email: self.inner.billing_email,
            invoicing_emails: self.inner.invoicing_emails.into_iter().map(Some).collect(),
            phone: self.inner.phone,
            balance_value_cents: self.inner.balance_value_cents,
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
    pub balance_value_cents: Option<i64>,
    pub currency: Option<String>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub billing_address: Option<Address>,
    #[map(~.map(|v| v.try_into()).transpose()?)]
    pub shipping_address: Option<ShippingAddress>,
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<Option<String>>,
    #[map(~.map(|v| serde_json::to_value(&v)).transpose().map_err(| e | {
    StoreError::SerdeError("Failed to serialize custom_taxes".to_string(), e)
    })?)]
    pub custom_taxes: Option<Vec<CustomerTaxRate>>,
    pub current_payment_method_id: Option<Option<CustomerPaymentMethodId>>,
    #[map(~.map(Into::into))]
    pub tax_status: Option<CustomerTaxStatus>,
    pub exemption_reason: Option<Option<String>>,
    pub connected_account_id: Option<Option<ConnectedAccountId>>,
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
pub struct CustomerTopUpBalance {
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub cents: i64,
    pub notes: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CustomerBuyCredits {
    pub created_by: Uuid,
    pub tenant_id: TenantId,
    pub customer_id: CustomerId,
    pub cents: i64,
    pub notes: Option<String>,
}

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
    pub custom_taxes: Vec<CustomerTaxRate>,
    pub tax_status: CustomerTaxStatus,
    pub exemption_reason: Option<String>,
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
    use super::*;
    use std::str::FromStr;

    fn customer_with(billing_email: Option<&str>, billing_address: Option<Address>) -> Customer {
        Customer {
            id: CustomerId::new(),
            name: "Acme".to_string(),
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: None,
            archived_at: None,
            tenant_id: TenantId::new(),
            invoicing_entity_id: InvoicingEntityId::new(),
            alias: None,
            billing_email: billing_email.map(str::to_string),
            phone: None,
            balance_value_cents: 0,
            currency: "EUR".to_string(),
            billing_address,
            shipping_address: None,
            current_payment_method_id: None,
            vat_number: None,
            invoicing_emails: vec![],
            conn_meta: None,
            tax_status: CustomerTaxStatus::Taxable,
            exemption_reason: None,
            custom_taxes: vec![],
            vat_number_format_valid: false,
            connected_account_id: None,
            vat_number_validation_status: None,
            vat_number_checked_at: None,
            vat_number_vies_check: None,
        }
    }

    fn complete_address() -> Address {
        Address {
            line1: Some("1 Rue de la Paix".to_string()),
            line2: None,
            city: Some("Berlin".to_string()),
            country: Some(CountryCode::from_str("DE").unwrap()),
            state: None,
            zip_code: Some("10115".to_string()),
        }
    }

    #[test]
    fn billing_information_complete_when_email_and_full_address_present() {
        let customer = customer_with(Some("billing@acme.com"), Some(complete_address()));
        assert!(customer.has_complete_billing_information());
    }

    #[test]
    fn billing_information_incomplete_without_email() {
        let customer = customer_with(None, Some(complete_address()));
        assert!(!customer.has_complete_billing_information());

        let blank_email = customer_with(Some("   "), Some(complete_address()));
        assert!(!blank_email.has_complete_billing_information());
    }

    #[test]
    fn billing_information_incomplete_without_address() {
        let customer = customer_with(Some("billing@acme.com"), None);
        assert!(!customer.has_complete_billing_information());
    }

    #[test]
    fn billing_information_incomplete_with_partial_address() {
        let missing_country = Address {
            country: None,
            ..complete_address()
        };
        assert!(
            !customer_with(Some("billing@acme.com"), Some(missing_country))
                .has_complete_billing_information()
        );

        let missing_zip = Address {
            zip_code: None,
            ..complete_address()
        };
        assert!(
            !customer_with(Some("billing@acme.com"), Some(missing_zip))
                .has_complete_billing_information()
        );

        let blank_city = Address {
            city: Some("  ".to_string()),
            ..complete_address()
        };
        assert!(
            !customer_with(Some("billing@acme.com"), Some(blank_city))
                .has_complete_billing_information()
        );
    }
}
