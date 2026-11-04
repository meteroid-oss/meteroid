use crate::client::PennylaneClient;
use crate::customer_invoices::{CustomerInvoiceLine, ImportedCustomerInvoice};
use crate::error::PennylaneError;
use crate::model::{ListResponse, QueryFilter, QueryParams};
use chrono::NaiveDate;
use reqwest::Method;
use secrecy::SecretString;
use serde::Serialize;

#[async_trait::async_trait]
pub trait CustomerCreditNotesApi {
    async fn import_customer_credit_note(
        &self,
        credit_note: &NewCustomerCreditNoteImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError>;

    async fn get_customer_credit_note(
        &self,
        external_reference: &str,
        access_token: &SecretString,
    ) -> Result<Option<ImportedCustomerInvoice>, PennylaneError>;

    async fn import_or_get_customer_credit_note(
        &self,
        credit_note: &NewCustomerCreditNoteImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError>;
}

#[async_trait::async_trait]
impl CustomerCreditNotesApi for PennylaneClient {
    /// <https://pennylane.readme.io/v2.0/reference/importcustomerinvoices> — v2 dropped the
    /// dedicated credit-note endpoints: an avoir is an invoice import with negative amounts
    /// and a `credited_invoice_id`.
    async fn import_customer_credit_note(
        &self,
        credit_note: &NewCustomerCreditNoteImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError> {
        self.execute(
            "/api/external/v2/customer_invoices/import",
            Method::POST,
            access_token,
            Some(credit_note),
            None::<()>.as_ref(),
        )
        .await
    }

    /// <https://app.pennylane.com/api/external/v2/customer_invoices>
    async fn get_customer_credit_note(
        &self,
        external_reference: &str,
        access_token: &SecretString,
    ) -> Result<Option<ImportedCustomerInvoice>, PennylaneError> {
        let list: ListResponse<ImportedCustomerInvoice> = self
            .execute(
                "/api/external/v2/customer_invoices",
                Method::GET,
                access_token,
                None::<()>.as_ref(),
                Some(&QueryParams {
                    filter: Some(vec![QueryFilter {
                        field: "external_reference".to_string(),
                        operator: "eq".to_string(),
                        value: external_reference.to_string(),
                    }]),
                }),
            )
            .await?;

        Ok(list.items.into_iter().next())
    }

    async fn import_or_get_customer_credit_note(
        &self,
        credit_note: &NewCustomerCreditNoteImport,
        access_token: &SecretString,
    ) -> Result<ImportedCustomerInvoice, PennylaneError> {
        let imported = self
            .import_customer_credit_note(credit_note, access_token)
            .await;

        if let Err(
            err @ PennylaneError::ClientError {
                error: _,
                status_code: Some(409 | 422),
            },
        ) = imported
        {
            self.get_customer_credit_note(credit_note.external_reference.as_ref(), access_token)
                .await?
                .ok_or(err)
        } else {
            imported
        }
    }
}

/// Amounts and lines must be negative — that is what makes the imported document an avoir.
#[derive(Debug, Clone, Serialize)]
pub struct NewCustomerCreditNoteImport {
    pub file_attachment_id: i64,
    pub customer_id: i64,
    pub external_reference: String, // meteroid credit note id
    pub invoice_number: Option<String>,
    pub date: NaiveDate,
    pub deadline: NaiveDate,
    pub currency: String,
    pub currency_amount_before_tax: String,
    pub currency_amount: String,
    pub currency_tax: String,
    /// Pennylane id of the customer invoice this avoir credits.
    pub credited_invoice_id: i64,
    pub invoice_lines: Vec<CustomerInvoiceLine>,
}
