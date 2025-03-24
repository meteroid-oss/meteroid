use crate::client::HubspotClient;
use crate::error::HubspotError;
use crate::model::{BatchUpsertItemRequest, BatchUpsertRequest, BatchUpsertResponse};
use common_domain::ids::{CustomerConnectionId, CustomerId};
use secrecy::SecretString;

#[async_trait::async_trait]
pub trait CompaniesApi {
    async fn batch_upsert_companies(
        &self,
        companies: Vec<NewCompany>,
        access_token: SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError>;
}

#[async_trait::async_trait]
impl CompaniesApi for HubspotClient {
    async fn batch_upsert_companies(
        &self,
        companies: Vec<NewCompany>,
        access_token: SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError> {
        self.batch_upsert(
            "/crm/v3/objects/companies/batch/upsert",
            BatchUpsertRequest {
                inputs: companies.into_iter().map(Into::into).collect(),
            },
            access_token,
        )
        .await
    }
}

pub struct NewCompany {
    pub customer_id: CustomerId,
    pub customer_connection_id: CustomerConnectionId,
    pub name: String,
    pub billing_email: Option<String>,
}

impl From<NewCompany> for BatchUpsertItemRequest {
    fn from(value: NewCompany) -> Self {
        BatchUpsertItemRequest {
            id: value.customer_connection_id.to_string(),
            id_property: Some("id".to_owned()),
            object_write_trace_id: None,
            properties: vec![
                ("name".to_owned(), Some(value.name)),
                ("email".to_owned(), value.billing_email),
                (
                    "meteroid_id".to_owned(),
                    Some(value.customer_id.to_string()),
                ),
            ],
        }
    }
}
