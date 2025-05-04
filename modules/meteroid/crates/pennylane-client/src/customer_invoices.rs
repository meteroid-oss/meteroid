use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use chrono::NaiveDate;
use reqwest::Method;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait CustomerInvoicesApi {
    async fn import_customer_invoice(
        &self,
        invoice: NewCustomerInvoiceImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError>;
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
