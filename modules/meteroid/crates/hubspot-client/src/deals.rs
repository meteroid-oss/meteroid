use crate::client::HubspotClient;
use crate::error::HubspotError;
use crate::model::{BatchActionRequest, BatchUpsertItemRequest, BatchUpsertResponse};
use chrono::NaiveDate;
use common_domain::ids::{CustomerId, SubscriptionId};
use secrecy::SecretString;
use serde_json::json;

#[async_trait::async_trait]
pub trait DealsApi {
    async fn upsert_deals(
        &self,
        deals: Vec<NewDeal>,
        access_token: &SecretString,
    ) -> Result<BatchUpsertResponse, HubspotError>;
}

#[async_trait::async_trait]
impl DealsApi for HubspotClient {
    /// https://developers.hubspot.com/docs/reference/api/crm/objects/deals#post-%2Fcrm%2Fv3%2Fobjects%2Fdeals%2Fbatch%2Fupsert
    async fn upsert_deals(
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
    pub customer_id: CustomerId,
    pub plan_name: String,
    pub customer_name: String,
    pub subscription_start_date: NaiveDate,
    pub subscription_end_date: Option<NaiveDate>,
    pub subscription_currency: String,
    pub subscription_mrr_cents: u64,
}

impl From<NewDeal> for BatchUpsertItemRequest {
    fn from(value: NewDeal) -> Self {
        BatchUpsertItemRequest {
            id: value.subscription_id.to_string(),
            id_property: Some("meteroid_subscription_id".to_owned()),
            object_write_trace_id: None,
            properties: json!({
               "dealname": value.plan_name,
                "meteroid_subscription_plan": value.plan_name,
                "meteroid_subscription_start_date": value.subscription_start_date.to_string(),
                "meteroid_subscription_end_date": value.subscription_end_date.map(|d| d.to_string()),
                "meteroid_subscription_currency": value.subscription_currency,
                "meteroid_subscription_mrr_cents": value.subscription_mrr_cents,
                "meteroid_subscription_id": value.subscription_id.to_string(),
                "meteroid_customer_id": value.customer_id.to_string(),
            }),
            // most of the time are ignored or fail silently in hubspot so managed by a separate API call
            associations: None,
        }
    }
}
