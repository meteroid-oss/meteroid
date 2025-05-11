use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use chrono::NaiveDate;
use reqwest::Method;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait CustomerInvoicesApi {
    async fn import_customer_invoice(
        &self,
        invoice: NewCustomerInvoiceImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError>;

    async fn mark_customer_invoice_as_paid(
        &self,
        invoice_id: i64,
        access_token: &SecretString,
    ) -> Result<(), PennylaneError>;
}

#[async_trait::async_trait]
impl CustomerInvoicesApi for PennylaneClient {
    /// https://pennylane.readme.io/v2.0/reference/importcustomerinvoices
    async fn import_customer_invoice(
        &self,
        invoice: NewCustomerInvoiceImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError> {
        self.execute(
            "/api/external/v2/customer_invoices/import",
            Method::POST,
            access_token,
            Some(invoice),
        )
        .await
    }

    /// https://pennylane.readme.io/v2.0/reference/markaspaidcustomerinvoice
    async fn mark_customer_invoice_as_paid(
        &self,
        invoice_id: i64,
        access_token: &SecretString,
    ) -> Result<(), PennylaneError> {
        let url = self
            .api_base
            .join(&format!(
                "/api/external/v2/customer_invoices/{invoice_id}/mark_as_paid"
            ))
            .expect("invalid path");

        let response = self
            .client
            .put(url)
            .bearer_auth(access_token.expose_secret())
            .send()
            .await
            .map_err(PennylaneError::from)?;

        let status_code = &response.status();

        if !status_code.is_success() {
            return Err(PennylaneError::ClientError {
                error: response.text().await.unwrap_or_default(),
                status_code: Some(status_code.as_u16()),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NewCustomerInvoiceImport {
    pub file_attachment_id: i64,
    pub customer_id: i64,
    pub external_reference: Option<String>,
    pub invoice_number: Option<String>,
    pub date: NaiveDate,
    pub deadline: NaiveDate,
    pub currency: String,
    pub currency_amount_before_tax: String,
    pub currency_amount: String,
    pub currency_tax: String,
    pub invoice_lines: Vec<CustomerInvoiceLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustomerInvoiceLine {
    pub currency_amount: String,
    pub currency_tax: String,
    pub label: String,
    pub quantity: Decimal,
    pub raw_currency_unit_price: String,
    pub unit: String,
    pub vat_rate: String, // an enum
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportedCustomerInvoice {
    pub id: i64,
}
