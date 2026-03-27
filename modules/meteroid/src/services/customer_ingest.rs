use common_domain::country::CountryCode;
use common_domain::ids::{InvoicingEntityId, string_serde_opt};
use meteroid_store::domain::{Address, ShippingAddress};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::Deserializer;

use super::csv_ingest::{
    CsvString, optional_bool, optional_country_code, optional_csv_string, optional_decimal,
};

#[derive(Debug, Clone, PartialEq)]
pub struct InvoicingEmails(pub Vec<String>);

impl<'de> Deserialize<'de> for InvoicingEmails {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s.is_empty() {
            Ok(InvoicingEmails(Vec::new()))
        } else {
            Ok(InvoicingEmails(
                s.split(',').map(|e| e.trim().to_string()).collect(),
            ))
        }
    }
}

#[derive(Deserialize)]
pub struct NewCustomerCsv {
    pub name: CsvString,
    #[serde(default, with = "optional_csv_string")]
    pub alias: Option<CsvString>,
    #[serde(default, with = "optional_csv_string")]
    pub billing_email: Option<CsvString>,
    #[serde(default)]
    pub invoicing_emails: Option<InvoicingEmails>,
    #[serde(default, with = "optional_csv_string")]
    pub phone: Option<CsvString>,
    pub currency: CsvString,
    #[serde(default, with = "string_serde_opt")]
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    #[serde(default, with = "optional_csv_string")]
    pub vat_number: Option<CsvString>,
    #[serde(flatten, default)]
    pub tax_rates: CustomTaxRatesCsv,
    #[serde(default, with = "optional_bool")]
    pub is_tax_exempt: Option<bool>,
    #[serde(flatten, default)]
    pub billing_address: AddressCsv,
    #[serde(flatten, default)]
    pub shipping_address: ShippingAddressCsv,
}

#[derive(Deserialize, Default)]
pub struct CustomTaxRatesCsv {
    #[serde(rename = "tax_rate1.tax_code", default, with = "optional_csv_string")]
    pub tax_code1: Option<CsvString>,
    #[serde(rename = "tax_rate1.name", default, with = "optional_csv_string")]
    pub name1: Option<CsvString>,
    #[serde(rename = "tax_rate1.rate", default, with = "optional_decimal")]
    pub rate1: Option<Decimal>,
    #[serde(rename = "tax_rate2.tax_code", default, with = "optional_csv_string")]
    pub tax_code2: Option<CsvString>,
    #[serde(rename = "tax_rate2.name", default, with = "optional_csv_string")]
    pub name2: Option<CsvString>,
    #[serde(rename = "tax_rate2.rate", default, with = "optional_decimal")]
    pub rate2: Option<Decimal>,
}

#[derive(Deserialize, Default)]
pub struct AddressCsv {
    #[serde(
        rename = "billing_address.line1",
        default,
        with = "optional_csv_string"
    )]
    pub line1: Option<CsvString>,
    #[serde(
        rename = "billing_address.line2",
        default,
        with = "optional_csv_string"
    )]
    pub line2: Option<CsvString>,
    #[serde(rename = "billing_address.city", default, with = "optional_csv_string")]
    pub city: Option<CsvString>,
    #[serde(
        rename = "billing_address.country",
        default,
        with = "optional_country_code"
    )]
    pub country: Option<CountryCode>,
    #[serde(
        rename = "billing_address.state",
        default,
        with = "optional_csv_string"
    )]
    pub state: Option<CsvString>,
    #[serde(
        rename = "billing_address.zip_code",
        default,
        with = "optional_csv_string"
    )]
    pub zip_code: Option<CsvString>,
}

impl From<AddressCsv> for Address {
    fn from(csv: AddressCsv) -> Self {
        Self {
            line1: csv.line1.map(|s| s.0),
            line2: csv.line2.map(|s| s.0),
            city: csv.city.map(|s| s.0),
            country: csv.country,
            state: csv.state.map(|s| s.0),
            zip_code: csv.zip_code.map(|s| s.0),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct ShippingAddressCsv {
    #[serde(
        rename = "shipping_address.same_as_billing",
        default,
        with = "optional_bool"
    )]
    pub same_as_billing: Option<bool>,
    #[serde(
        rename = "shipping_address.line1",
        default,
        with = "optional_csv_string"
    )]
    pub line1: Option<CsvString>,
    #[serde(
        rename = "shipping_address.line2",
        default,
        with = "optional_csv_string"
    )]
    pub line2: Option<CsvString>,
    #[serde(
        rename = "shipping_address.city",
        default,
        with = "optional_csv_string"
    )]
    pub city: Option<CsvString>,
    #[serde(
        rename = "shipping_address.country",
        default,
        with = "optional_country_code"
    )]
    pub country: Option<CountryCode>,
    #[serde(
        rename = "shipping_address.state",
        default,
        with = "optional_csv_string"
    )]
    pub state: Option<CsvString>,
    #[serde(
        rename = "shipping_address.zip_code",
        default,
        with = "optional_csv_string"
    )]
    pub zip_code: Option<CsvString>,
}

impl From<ShippingAddressCsv> for ShippingAddress {
    fn from(csv: ShippingAddressCsv) -> Self {
        Self {
            same_as_billing: csv.same_as_billing.unwrap_or(true),
            address: Some(Address {
                line1: csv.line1.map(|s| s.0),
                line2: csv.line2.map(|s| s.0),
                city: csv.city.map(|s| s.0),
                country: csv.country,
                state: csv.state.map(|s| s.0),
                zip_code: csv.zip_code.map(|s| s.0),
            }),
        }
    }
}
