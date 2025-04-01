use crate::client::HubspotClient;
use crate::error::HubspotError;
use crate::model::{BatchActionRequest, BatchUpsertItemRequest, BatchUpsertResponse};
use common_domain::ids::CustomerId;
use secrecy::SecretString;
use serde_json::json;

#[async_trait::async_trait]
pub trait CompaniesApi {
    async fn batch_upsert_companies(
        &self,
        companies: Vec<NewCompany>,
        access_token: &SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError>;
}

#[async_trait::async_trait]
impl CompaniesApi for HubspotClient {
    /// https://developers.hubspot.com/docs/reference/api/crm/objects/companies#post-%2Fcrm%2Fv3%2Fobjects%2Fcompanies%2Fbatch%2Fupsert
    async fn batch_upsert_companies(
        &self,
        companies: Vec<NewCompany>,
        access_token: &SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError> {
        self.batch_upsert(
            "/crm/v3/objects/companies/batch/upsert",
            BatchActionRequest {
                inputs: companies.into_iter().map(Into::into).collect(),
            },
            access_token,
        )
        .await
    }
}

pub struct NewCompany {
    pub customer_id: CustomerId,
    pub name: String,
    pub billing_email: Option<String>,
    pub billing_address: Option<CompanyAddress>,
}

impl From<NewCompany> for BatchUpsertItemRequest {
    fn from(value: NewCompany) -> Self {
        BatchUpsertItemRequest {
            id: value.customer_id.to_string(),
            id_property: Some("meteroid_customer_id".to_owned()),
            object_write_trace_id: None,
            properties: json!({
                "name": value.name,
                "meteroid_customer_id": value.customer_id.to_string(),
            }),
        }
    }
}

pub struct CompanyAddress {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub state: Option<String>,
    pub zip_code: Option<String>,
}
