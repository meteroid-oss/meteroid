use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use crate::model::{ListResponse, QueryFilter, QueryParams};
use reqwest::Method;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[async_trait::async_trait]
pub trait CustomersApi {
    async fn create_company_customer(
        &self,
        company: &NewCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError>;

    async fn update_company_customer(
        &self,
        id: i64,
        company: &UpdateCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError>;

    async fn upsert_company_customer(
        &self,
        company: &NewCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError>;

    async fn get_company_customer(
        &self,
        external_reference: &str,
        access_token: &SecretString,
    ) -> Result<Option<Company>, PennylaneError>;
}

#[async_trait::async_trait]
impl CustomersApi for PennylaneClient {
    /// <https://pennylane.readme.io/v2.0/reference/postcompanycustomer>
    async fn create_company_customer(
        &self,
        company: &NewCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError> {
        self.execute(
            "/api/external/v2/company_customers",
            Method::POST,
            access_token,
            Some(company),
            None::<()>.as_ref(),
        )
        .await
    }

    /// <https://pennylane.readme.io/v2.0/reference/putcompanycustomer>
    async fn update_company_customer(
        &self,
        id: i64,
        company: &UpdateCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError> {
        self.execute(
            format!("/api/external/v2/company_customers/{id}").as_str(),
            Method::PUT,
            access_token,
            Some(company),
            None::<()>.as_ref(),
        )
        .await
    }

    async fn upsert_company_customer(
        &self,
        company: &NewCompany,
        access_token: &SecretString,
    ) -> Result<Company, PennylaneError> {
        let created = self.create_company_customer(company, access_token).await;

        if let Err(
            err @ PennylaneError::ClientError {
                error: _,
                status_code: Some(409 | 422),
            },
        ) = created
        {
            let existing = self
                .get_company_customer(company.external_reference.as_str(), access_token)
                .await?
                .ok_or(err)?;

            self.update_company_customer(existing.id, &company.into(), access_token)
                .await
        } else {
            created
        }
    }

    /// <https://pennylane.readme.io/v2.0/reference/getcustomers>
    async fn get_company_customer(
        &self,
        external_reference: &str,
        access_token: &SecretString,
    ) -> Result<Option<Company>, PennylaneError> {
        let list: ListResponse<Company> = self
            .execute(
                "/api/external/v2/customers",
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
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct NewCompany {
    pub name: String,
    pub billing_address: BillingAddress,
    pub phone: Option<String>,
    pub external_reference: String, // meteroid customer id
    pub vat_number: Option<String>,
    pub emails: Vec<String>, // invoicing_emails
    pub billing_iban: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCompany {
    pub name: String,
    pub billing_address: BillingAddress,
    pub phone: Option<String>,
    pub external_reference: String, // meteroid customer id
    pub vat_number: Option<String>,
    pub emails: Vec<String>, // invoicing_emails
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

impl From<&NewCompany> for UpdateCompany {
    fn from(new: &NewCompany) -> Self {
        UpdateCompany {
            name: new.name.clone(),
            billing_address: new.billing_address.clone(),
            phone: new.phone.clone(),
            external_reference: new.external_reference.clone(),
            vat_number: new.vat_number.clone(),
            emails: new.emails.clone(),
            billing_iban: new.billing_iban.clone(),
        }
    }
}
