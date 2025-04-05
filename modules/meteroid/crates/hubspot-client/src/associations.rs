use crate::client::HubspotClient;
use crate::error::HubspotError;
use crate::model::{
    Associate, Association, AssociationCategory, AssociationType, AssociationTypeId,
    BatchActionRequest, CompanyId, DealId, StandardErrorResponse,
};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Deserialize;

#[async_trait::async_trait]
pub trait AssociationsApi {
    async fn associate_deals_to_companies(
        &self,
        associations: Vec<(DealId, CompanyId)>,
        access_token: &SecretString,
    ) -> Result<BatchAssociateResponse, HubspotError>;
}

#[async_trait::async_trait]
impl AssociationsApi for HubspotClient {
    /// https://developers.hubspot.com/docs/reference/api/crm/associations/association-details#post-%2Fcrm%2Fv4%2Fassociations%2F%7Bfromobjecttype%7D%2F%7Btoobjecttype%7D%2Fbatch%2Fcreate
    async fn associate_deals_to_companies(
        &self,
        associations: Vec<(DealId, CompanyId)>,
        access_token: &SecretString,
    ) -> Result<BatchAssociateResponse, HubspotError> {
        self.execute(
            "/crm/v4/associations/deals/companies/batch/create",
            reqwest::Method::POST,
            access_token,
            Some(BatchActionRequest {
                inputs: associations
                    .into_iter()
                    .map(|(deal_id, company_id)| Association {
                        from: Some(Associate { id: deal_id.0 }),
                        to: Associate { id: company_id.0 },
                        types: vec![AssociationType {
                            association_category: AssociationCategory::HubspotDefined,
                            association_type_id: AssociationTypeId::DealToCompany,
                        }],
                    })
                    .collect(),
            }),
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchAssociateResponse {
    #[serde(rename = "completedAt")]
    pub completed_at: DateTime<Utc>,
    #[serde(rename = "startedAt")]
    pub started_at: DateTime<Utc>,
    pub status: String,
    pub results: Vec<serde_json::Value>,
    #[serde(rename = "numErrors")]
    pub num_errors: Option<i32>, // for status_207 status responses (multiple statuses)
    pub errors: Option<Vec<StandardErrorResponse>>, // for status_207 responses (multiple statuses)
}
