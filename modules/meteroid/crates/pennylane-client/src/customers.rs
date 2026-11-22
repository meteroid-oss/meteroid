use crate::client::PennylaneClient;
use crate::error::PennylaneError;
use crate::model::{ListQuery, ListResponse};
use reqwest::Method;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Address {
    pub address: String,
    pub postal_code: String,
    pub city: String,
    pub country_alpha2: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BillingLanguage {
    #[serde(rename = "fr_FR")]
    French,
    #[serde(rename = "en_GB")]
    English,
    #[serde(rename = "de_DE")]
    German,
    #[serde(rename = "es_ES")]
    Spanish,
}

/// Body of both the creation and the update of a company customer.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompanyCustomer {
    pub name: String,
    pub vat_number: Option<String>,
    pub reg_no: Option<String>,
    pub phone: Option<String>,
    pub billing_address: Address,
    pub delivery_address: Option<Address>,
    pub emails: Vec<String>,
    pub external_reference: String,
    pub billing_language: Option<BillingLanguage>,
}

/// Body of both the creation and the update of an individual customer.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndividualCustomer {
    pub first_name: String,
    pub last_name: String,
    pub phone: Option<String>,
    pub billing_address: Address,
    pub delivery_address: Option<Address>,
    pub emails: Vec<String>,
    pub external_reference: String,
    pub billing_language: Option<BillingLanguage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomerUpsert {
    Company(CompanyCustomer),
    Individual(IndividualCustomer),
}

impl CustomerUpsert {
    pub fn external_reference(&self) -> &str {
        match self {
            CustomerUpsert::Company(company) => &company.external_reference,
            CustomerUpsert::Individual(individual) => &individual.external_reference,
        }
    }

    pub fn customer_type(&self) -> CustomerType {
        match self {
            CustomerUpsert::Company(_) => CustomerType::Company,
            CustomerUpsert::Individual(_) => CustomerType::Individual,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomerType {
    Company,
    Individual,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Customer {
    pub id: i64,
    #[serde(default)]
    pub external_reference: Option<String>,
    /// Only set by the list endpoint.
    #[serde(default)]
    pub customer_type: Option<CustomerType>,
}

impl PennylaneClient {
    /// <https://pennylane.readme.io/reference/postcompanycustomer>,
    /// <https://pennylane.readme.io/reference/postindividualcustomer>
    pub async fn create_customer(
        &self,
        customer: &CustomerUpsert,
        access_token: &SecretString,
    ) -> Result<Customer, PennylaneError> {
        match customer {
            CustomerUpsert::Company(body) => {
                self.send_json(
                    Method::POST,
                    "/api/external/v2/company_customers",
                    access_token,
                    body,
                )
                .await
            }
            CustomerUpsert::Individual(body) => {
                self.send_json(
                    Method::POST,
                    "/api/external/v2/individual_customers",
                    access_token,
                    body,
                )
                .await
            }
        }
    }

    /// <https://pennylane.readme.io/reference/putcompanycustomer>,
    /// <https://pennylane.readme.io/reference/putindividualcustomer>
    pub async fn update_customer(
        &self,
        id: i64,
        customer: &CustomerUpsert,
        access_token: &SecretString,
    ) -> Result<Customer, PennylaneError> {
        match customer {
            CustomerUpsert::Company(body) => {
                self.send_json(
                    Method::PUT,
                    &format!("/api/external/v2/company_customers/{id}"),
                    access_token,
                    body,
                )
                .await
            }
            CustomerUpsert::Individual(body) => {
                self.send_json(
                    Method::PUT,
                    &format!("/api/external/v2/individual_customers/{id}"),
                    access_token,
                    body,
                )
                .await
            }
        }
    }

    /// <https://pennylane.readme.io/reference/getcustomers>
    pub async fn find_customer(
        &self,
        external_reference: &str,
        access_token: &SecretString,
    ) -> Result<Option<Customer>, PennylaneError> {
        let list: ListResponse<Customer> = self
            .get(
                "/api/external/v2/customers",
                access_token,
                &ListQuery::eq("external_reference", external_reference),
            )
            .await?;
        Ok(list.items.into_iter().next())
    }
}
