use common_domain::country::CountryCode;
use common_domain::ids::{InvoicingEntityId, TenantId, string_serde_opt};
use csv::ReaderBuilder;
use error_stack::bail;
use meteroid_store::domain::{Address, CustomerCustomTax, CustomerNew, ShippingAddress};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::{Store, StoreResult};
use serde::Deserialize;
use serde::de::{Deserializer, Visitor};
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use uuid::Uuid;

const MAX_CSV_SIZE: usize = 10 * 1024 * 1024; // 10MB limit
const MAX_BATCH_SIZE: usize = 500; // Max events per batch

#[derive(Clone)]
pub struct CustomerIngestService {
    store: Arc<Store>,
}

impl CustomerIngestService {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub async fn ingest_csv(
        &self,
        tenant_id: TenantId,
        actor: Uuid,
        file_data: &[u8],
        options: CsvIngestionOptions,
    ) -> StoreResult<CsvIngestionResult> {
        let (parsed, mut failures) = Self::parse_csv(actor, file_data, options.delimiter as u8)?;

        let total_rows = (parsed.len() + failures.len()) as i32;

        // If fail_on_error is true, and we have parsing failures, return error immediately
        if options.fail_on_error && !failures.is_empty() {
            return Ok(CsvIngestionResult {
                total_rows,
                successful_rows: 0,
                failures,
            });
        }

        tracing::info!(
            "Processing {} customer records in {} batches",
            parsed.len(),
            parsed.len().div_ceil(MAX_BATCH_SIZE)
        );

        let mut successful_rows = 0;

        for (batch_idx, chunk) in parsed.chunks(MAX_BATCH_SIZE).enumerate() {
            tracing::info!(
                "Saving batch {} with {} customers",
                batch_idx + 1,
                chunk.len()
            );

            let result = self
                .store
                .upsert_customer_batch(chunk.to_vec(), tenant_id)
                .await;

            if let Err(e) = result {
                // If batch fails, record failures for all rows in the batch
                for cus in chunk {
                    failures.push(CsvIngestionFailure {
                        row_number: -1,
                        alias: cus.alias.clone().unwrap_or(String::new()),
                        reason: format!("Database error: {}", e),
                    });
                }

                // If fail_on_error is true, stop processing further batches
                if options.fail_on_error {
                    break;
                }
            } else {
                successful_rows += chunk.len() as i32;
            }
        }

        Ok(CsvIngestionResult {
            total_rows,
            successful_rows,
            failures,
        })
    }

    pub fn parse_csv(
        actor: Uuid,
        file_data: &[u8],
        delimiter: u8,
    ) -> StoreResult<(Vec<CustomerNew>, Vec<CsvIngestionFailure>)> {
        if file_data.is_empty() {
            bail!(StoreError::InvalidArgument("File is empty".to_string()));
        }

        if file_data.len() > MAX_CSV_SIZE {
            bail!(StoreError::InvalidArgument(format!(
                "File size exceeds maximum allowed ({MAX_CSV_SIZE} bytes)"
            )));
        }

        let mut reader = ReaderBuilder::new()
            .has_headers(true)
            .delimiter(delimiter)
            .from_reader(file_data);

        let mut parsed = Vec::new();
        let mut failures = Vec::new();
        let mut row_number = 2; // Account for header row (always present)

        let iter = reader.deserialize::<NewCustomerCsv>();

        for rec in iter {
            match rec {
                Ok(cus) => match Self::map_to_domain(actor, cus) {
                    Ok(c) => parsed.push(c),
                    Err(e) => failures.push(CsvIngestionFailure {
                        row_number,
                        alias: String::new(),
                        reason: format!("Failed to convert to domain: {e}"),
                    }),
                },
                Err(e) => failures.push(CsvIngestionFailure {
                    row_number,
                    alias: String::new(),
                    reason: format!("Failed to parse row: {e}"),
                }),
            }
            row_number += 1;
        }

        Ok((parsed, failures))
    }

    fn map_to_domain(actor: Uuid, csv: NewCustomerCsv) -> Result<CustomerNew, String> {
        let billing_address = if csv.billing_address.country.is_some() {
            Some(csv.billing_address.into())
        } else {
            None
        };

        let shipping_address = if csv.shipping_address.country.is_some() {
            Some(csv.shipping_address.into())
        } else {
            None
        };

        let custom_taxes = Self::parse_tax_rates(&csv.tax_rates)?;

        Ok(CustomerNew {
            name: csv.name.0,
            created_by: actor,
            alias: csv.alias.map(|a| a.0),
            billing_email: csv.billing_email.map(|e| e.0),
            invoicing_emails: csv.invoicing_emails.0,
            phone: csv.phone.map(|p| p.0),
            balance_value_cents: 0,
            currency: csv.currency.0,
            billing_address,
            shipping_address,
            force_created_date: None,
            bank_account_id: None,
            vat_number: csv.vat_number.map(|v| v.0),
            invoicing_entity_id: csv.invoicing_entity_id,
            custom_taxes,
            is_tax_exempt: csv.is_tax_exempt.unwrap_or(false),
        })
    }

    fn parse_tax_rates(tax_rates: &CustomTaxRatesCsv) -> Result<Vec<CustomerCustomTax>, String> {
        let mut taxes = Vec::new();

        if let Some(rate) = tax_rates.rate1 {
            taxes.push(Self::parse_tax_rate(
                rate,
                &tax_rates.tax_code1,
                &tax_rates.name1,
                "tax_rate1",
            )?);
        }

        if let Some(rate) = tax_rates.rate2 {
            taxes.push(Self::parse_tax_rate(
                rate,
                &tax_rates.tax_code2,
                &tax_rates.name2,
                "tax_rate2",
            )?);
        }

        Ok(taxes)
    }

    fn parse_tax_rate(
        rate: rust_decimal::Decimal,
        tax_code: &Option<CsvString>,
        name: &Option<CsvString>,
        field_name: &str,
    ) -> Result<CustomerCustomTax, String> {
        let tax_code = tax_code
            .as_ref()
            .ok_or(format!(
                "{field_name}.tax_code is required if rate is provided"
            ))?
            .0
            .clone();

        let name = name
            .as_ref()
            .ok_or(format!("{field_name}.name is required if rate is provided"))?
            .0
            .clone();

        Ok(CustomerCustomTax {
            tax_code,
            name,
            rate,
        })
    }
}

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
    pub alias: Option<CsvString>,
    pub billing_email: Option<CsvString>,
    pub invoicing_emails: InvoicingEmails,
    pub phone: Option<CsvString>,
    pub currency: CsvString,
    #[serde(default, with = "string_serde_opt")]
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    pub vat_number: Option<CsvString>,
    #[serde(flatten)]
    pub tax_rates: CustomTaxRatesCsv,
    pub is_tax_exempt: Option<bool>,
    #[serde(flatten)]
    pub billing_address: AddressCsv,
    #[serde(flatten)]
    pub shipping_address: ShippingAddressCsv,
}

#[derive(Deserialize)]
pub struct CustomTaxRatesCsv {
    #[serde(rename = "tax_rate1.tax_code")]
    pub tax_code1: Option<CsvString>,
    #[serde(rename = "tax_rate1.name")]
    pub name1: Option<CsvString>,
    #[serde(rename = "tax_rate1.rate", with = "rust_decimal::serde::float_option")]
    pub rate1: Option<rust_decimal::Decimal>,
    #[serde(rename = "tax_rate2.tax_code")]
    pub tax_code2: Option<CsvString>,
    #[serde(rename = "tax_rate2.name")]
    pub name2: Option<CsvString>,
    #[serde(rename = "tax_rate2.rate", with = "rust_decimal::serde::float_option")]
    pub rate2: Option<rust_decimal::Decimal>,
}

#[derive(Deserialize)]
pub struct AddressCsv {
    #[serde(rename = "billing_address.line1")]
    pub line1: Option<CsvString>,
    #[serde(rename = "billing_address.line2")]
    pub line2: Option<CsvString>,
    #[serde(rename = "billing_address.city")]
    pub city: Option<CsvString>,
    #[serde(rename = "billing_address.country")]
    pub country: Option<CountryCode>,
    #[serde(rename = "billing_address.state")]
    pub state: Option<CsvString>,
    #[serde(rename = "billing_address.zip_code")]
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

#[derive(Deserialize)]
pub struct ShippingAddressCsv {
    #[serde(rename = "shipping_address.same_as_billing")]
    pub same_as_billing: Option<bool>,
    #[serde(rename = "shipping_address.line1")]
    pub line1: Option<CsvString>,
    #[serde(rename = "shipping_address.line2")]
    pub line2: Option<CsvString>,
    #[serde(rename = "shipping_address.city")]
    pub city: Option<CsvString>,
    #[serde(rename = "shipping_address.country")]
    pub country: Option<CountryCode>,
    #[serde(rename = "shipping_address.state")]
    pub state: Option<CsvString>,
    #[serde(rename = "shipping_address.zip_code")]
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

/// A string-like type that accepts numbers or strings during deserialization.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CsvString(pub String);

impl Deref for CsvString {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&str> for CsvString {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<CsvString> for String {
    fn from(f: CsvString) -> Self {
        f.0
    }
}

impl fmt::Display for CsvString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'de> Deserialize<'de> for CsvString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CsvStringVisitor;

        impl<'de> Visitor<'de> for CsvStringVisitor {
            type Value = CsvString;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string or a primitive convertible to string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(CsvString(v))
            }

            // Numbers -> string
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }

            // Bool -> "true"/"false"
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(CsvString(v.to_string()))
            }
        }

        deserializer.deserialize_any(CsvStringVisitor)
    }
}

#[derive(Debug, Clone)]
pub struct CsvIngestionOptions {
    pub delimiter: char,
    pub allow_backfilling: bool,
    pub fail_on_error: bool,
}

#[derive(Debug, Clone)]
pub struct CsvIngestionFailure {
    pub row_number: i32,
    pub alias: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct CsvIngestionResult {
    pub total_rows: i32,
    pub successful_rows: i32,
    pub failures: Vec<CsvIngestionFailure>,
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    #[test]
    fn parse_csv_test() {
        let raw_csv = r#"name;alias;billing_email;invoicing_emails;phone;currency;invoicing_entity_id;vat_number;tax_rate1.tax_code;tax_rate1.name;tax_rate1.rate;tax_rate2.tax_code;tax_rate2.name;tax_rate2.rate;is_tax_exempt;billing_address.line1;billing_address.line2;billing_address.city;billing_address.country;billing_address.state;billing_address.zip_code;shipping_address.same_as_billing;shipping_address.line1;shipping_address.line2;shipping_address.city;shipping_address.country;shipping_address.state;shipping_address.zip_code
Acme Corp;acme;billing@acme.com;invoices@acme.com,accounting@acme.com;+1234567890;USD;ive_7n42DGM5Tflk9n8mt7Fhc7;FR12345678901;VAT;Value Added Tax;0.20;GST;Goods and Services Tax;0.05;false;123 Main St;Suite 100;New York;US;NY;10001;true;;;;;;"#;

        let (parsed, failures) =
            super::CustomerIngestService::parse_csv(Uuid::default(), raw_csv.as_bytes(), b';')
                .unwrap();

        assert_eq!(failures.len(), 0);
        assert_eq!(parsed.len(), 1);

        let customer = &parsed[0];
        assert_eq!(customer.name, "Acme Corp".to_string());
        assert_eq!(customer.alias.as_deref(), Some("acme"));
        assert_eq!(
            customer.invoicing_emails,
            vec!["invoices@acme.com", "accounting@acme.com"]
        );
        assert_eq!(
            customer
                .billing_address
                .as_ref()
                .and_then(|a| a.zip_code.as_ref())
                .map(String::as_str),
            Some("10001")
        );
    }
}
