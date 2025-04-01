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
                "meteroid_customer_email": value.billing_email,
                "meteroid_customer_country": value.billing_address.as_ref().and_then(|v| v.country.as_ref()),
                "meteroid_customer_city": value.billing_address.as_ref().and_then(|v| v.city.as_ref()),
                "meteroid_customer_state": value.billing_address.as_ref().and_then(|v| v.state.as_ref()),
                "meteroid_customer_street": value.billing_address.as_ref().and_then(|v| v.line1.as_ref()),
                "meteroid_customer_postal_code": value.billing_address.as_ref().and_then(|v| v.zip_code.as_ref()),
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
