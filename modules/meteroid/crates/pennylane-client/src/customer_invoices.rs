use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use crate::model::{ListQuery, ListResponse};
use crate::vat_rate::VatRate;
use chrono::NaiveDate;
use reqwest::Method;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize, Serializer};
use serde_with::skip_serializing_none;

/// A monetary amount, sent as the decimal string Pennylane stores as is (`"-12.30"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(pub Decimal);

impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

/// A customer invoice or, with negative amounts and `credited_invoice_id`, a credit note.
///
/// Pennylane rejects a document whose figures do not add up: `currency_amount_before_tax +
/// currency_tax = currency_amount`, and the lines' `currency_amount` and `currency_tax` sum to the
/// document's.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CustomerInvoiceImport {
    pub file_attachment_id: i64,
    pub customer_id: i64,
    pub external_reference: String,
    pub invoice_number: String,
    pub date: NaiveDate,
    pub deadline: NaiveDate,
    pub currency: String,
    pub currency_amount_before_tax: Amount,
    pub currency_amount: Amount,
    pub currency_tax: Amount,
    /// The total in euros, for a document in another currency; Pennylane converts it otherwise.
    pub amount: Option<Amount>,
    pub tax: Option<Amount>,
    pub credited_invoice_id: Option<i64>,
    pub transaction_reference: Option<TransactionReference>,
    /// At most 50 characters.
    pub purchase_order_reference: Option<String>,
    pub invoice_lines: Vec<InvoiceLine>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InvoiceLine {
    pub label: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub quantity: Decimal,
    /// Excluding tax, at most 6 decimals.
    pub raw_currency_unit_price: String,
    pub unit: String,
    pub vat_rate: VatRate,
    /// Including tax.
    pub currency_amount: Amount,
    pub currency_tax: Amount,
    pub amount: Option<Amount>,
    pub tax: Option<Amount>,
    pub imputation_dates: Option<ImputationDates>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImputationDates {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

/// Lets Pennylane reconcile the invoice with the transaction it receives from its own bank or
/// payment provider feed: <https://pennylane.readme.io/docs/automating-payment-matching>.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionReference {
    pub banking_provider: BankingProvider,
    pub provider_field_name: ProviderFieldName,
    pub provider_field_value: String,
}

impl TransactionReference {
    /// A bank transfer whose label carries the invoice number.
    pub fn bank_label(invoice_number: &str) -> Self {
        Self {
            banking_provider: BankingProvider::Bank,
            provider_field_name: ProviderFieldName::Label,
            provider_field_value: invoice_number.to_owned(),
        }
    }

    pub fn stripe_payment(payment_intent_id: &str) -> Self {
        Self {
            banking_provider: BankingProvider::Stripe,
            provider_field_name: ProviderFieldName::PaymentId,
            provider_field_value: payment_intent_id.to_owned(),
        }
    }

    /// A Stripe charge (`ch_`) or non-card payment (`py_`) without a payment intent.
    pub fn stripe_charge(charge_id: &str) -> Self {
        Self {
            banking_provider: BankingProvider::Stripe,
            provider_field_name: ProviderFieldName::ChargeId,
            provider_field_value: charge_id.to_owned(),
        }
    }

    pub fn gocardless_payment(payment_id: &str) -> Self {
        Self {
            banking_provider: BankingProvider::Gocardless,
            provider_field_name: ProviderFieldName::PaymentId,
            provider_field_value: payment_id.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BankingProvider {
    Stripe,
    Gocardless,
    Bank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFieldName {
    PaymentId,
    ChargeId,
    Label,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportedInvoiceUpdate {
    pub transaction_reference: TransactionReference,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomerInvoice {
    pub id: i64,
    #[serde(default)]
    pub external_reference: Option<String>,
    #[serde(default)]
    pub paid: bool,
    #[serde(default)]
    pub transaction_reference: Option<serde_json::Value>,
}

impl PennylaneClient {
    /// <https://pennylane.readme.io/reference/importcustomerinvoices>
    pub async fn import_customer_invoice(
        &self,
        invoice: &CustomerInvoiceImport,
        access_token: &SecretString,
    ) -> Result<CustomerInvoice, PennylaneError> {
        self.send_json(
            Method::POST,
            "/api/external/v2/customer_invoices/import",
            access_token,
            invoice,
        )
        .await
    }

    /// <https://pennylane.readme.io/reference/getcustomerinvoices>: invoices and credit notes.
    pub async fn find_customer_invoice(
        &self,
        external_reference: &str,
        access_token: &SecretString,
    ) -> Result<Option<CustomerInvoice>, PennylaneError> {
        let list: ListResponse<CustomerInvoice> = self
            .get(
                "/api/external/v2/customer_invoices",
                access_token,
                &ListQuery::eq("external_reference", external_reference),
            )
            .await?;
        Ok(list.items.into_iter().next())
    }

    /// <https://pennylane.readme.io/reference/getcustomerinvoice>
    pub async fn get_customer_invoice(
        &self,
        id: i64,
        access_token: &SecretString,
    ) -> Result<CustomerInvoice, PennylaneError> {
        self.get(
            &format!("/api/external/v2/customer_invoices/{id}"),
            access_token,
            &(),
        )
        .await
    }

    /// <https://pennylane.readme.io/reference/updateimportedcustomerinvoice>
    pub async fn update_imported_customer_invoice(
        &self,
        id: i64,
        update: &ImportedInvoiceUpdate,
        access_token: &SecretString,
    ) -> Result<CustomerInvoice, PennylaneError> {
        self.send_json(
            Method::PUT,
            &format!("/api/external/v2/customer_invoices/{id}/update_imported"),
            access_token,
            update,
        )
        .await
    }

    /// <https://pennylane.readme.io/reference/markaspaidcustomerinvoice>: no reconciliation is
    /// attempted once marked.
    pub async fn mark_customer_invoice_as_paid(
        &self,
        id: i64,
        access_token: &SecretString,
    ) -> Result<(), PennylaneError> {
        self.put_empty(
            &format!("/api/external/v2/customer_invoices/{id}/mark_as_paid"),
            access_token,
        )
        .await
    }
}
