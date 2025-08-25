use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use reqwest::Method;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait CustomersApi {
    async fn create_company_customer(
        &self,
        company: NewCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError>;

    async fn update_company_customer(
        &self,
        id: i64,
        company: UpdateCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError>;
}

#[async_trait::async_trait]
impl CustomersApi for PennylaneClient {
    /// https://pennylane.readme.io/v2.0/reference/postcompanycustomer
    async fn create_company_customer(
        &self,
        company: NewCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError> {
        self.execute(
            "/api/external/v2/company_customers",
            Method::POST,
            access_token,
            Some(company),
        )
        .await
    }

    /// https://pennylane.readme.io/v2.0/reference/putcompanycustomer
    async fn update_company_customer(
        &self,
        id: i64,
        company: UpdateCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError> {
        self.execute(
            format!("/api/external/v2/company_customers/{id}").as_str(),
            Method::PUT,
            access_token,
            Some(company),
        )
        .await
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NewCompany {
    pub name: String,
    pub billing_address: BillingAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    pub external_reference: String, // meteroid customer id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    pub emails: Vec<String>, // invoicing_emails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_iban: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCompany {
    pub name: String,
    pub billing_address: BillingAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    pub external_reference: String, // meteroid customer id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vat_number: Option<String>,
    pub emails: Vec<String>, // invoicing_emails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_iban: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingAddress {
    pub address: String,
    pub postal_code: String,
    pub city: String,
    pub country_alpha2: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Company {
    pub id: i64,
    pub name: String,
    pub billing_address: BillingAddress,
    pub phone: Option<String>,
    pub reference: Option<String>, // meteroid customer id
    pub vat_number: Option<String>,
    pub emails: Vec<String>, // invoicing_emails
    pub billing_iban: Option<String>,
}
