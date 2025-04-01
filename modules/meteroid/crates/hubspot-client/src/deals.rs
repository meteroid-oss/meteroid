use crate::client::HubspotClient;
use crate::error::HubspotError;
use crate::model::{BatchActionRequest, BatchUpsertItemRequest, BatchUpsertResponse};
use chrono::NaiveDate;
use common_domain::ids::SubscriptionId;
use secrecy::SecretString;
use serde_json::json;

#[async_trait::async_trait]
pub trait DealsApi {
    async fn batch_upsert_deals(
        &self,
        deals: Vec<NewDeal>,
        access_token: &SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError>;
}

#[async_trait::async_trait]
impl DealsApi for HubspotClient {
    async fn batch_upsert_deals(
        &self,
        deals: Vec<NewDeal>,
        access_token: &SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError> {
        self.batch_upsert(
            "/crm/v3/objects/deals/batch/upsert",
            BatchActionRequest {
                inputs: deals.into_iter().map(Into::into).collect(),
            },
            access_token,
        )
        .await
    }
}

pub struct NewDeal {
    pub subscription_id: SubscriptionId,
    pub plan_name: String,
    pub customer_name: String,
    pub subscription_start_date: NaiveDate,
    pub subscription_end_date: Option<NaiveDate>,
    pub subscription_status: String,
    pub subscription_currency: String,
    pub subscription_mrr: u64,
}

impl From<NewDeal> for BatchUpsertItemRequest {
    fn from(value: NewDeal) -> Self {
        BatchUpsertItemRequest {
            id: value.subscription_id.to_string(), // todo confirm me
            id_property: Some("id".to_owned()),    // todo confirm me
            object_write_trace_id: None,
            properties: json!({
               "name": value.customer_name,
                "meteroid_subscription_plan": value.plan_name,
                "meteroid_subscription_start_date": value.subscription_start_date.to_string(),
                "meteroid_subscription_end_date": value.subscription_end_date.map(|d| d.to_string()),
                "meteroid_subscription_currency": value.subscription_currency,
                "meteroid_subscription_mrr": value.subscription_mrr.to_string(),
                "meteroid_subscription_status": value.subscription_status,
                "meteroid_subscription_id": value.subscription_id.to_string(),
            }),
        }
    }
}
