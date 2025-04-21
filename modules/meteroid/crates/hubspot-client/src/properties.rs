use crate::client::HubspotClient;
use crate::error::HubspotError;
use crate::model::{BatchActionRequest, StandardErrorResponse};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[async_trait::async_trait]
pub trait PropertiesApi {
    async fn batch_create_properties(
        &self,
        object_type: ObjectType,
        properties: Vec<NewProperty>,
        access_token: &SecretString,
    ) -> Result<BatchCreateResponse, HubspotError>;

    async fn create_property_group(
        &self,
        object_type: ObjectType,
        property_group: NewPropertyGroup,
        access_token: &SecretString,
    ) -> Result<(), HubspotError>;

    async fn create_meteroid_properties(
        &self,
        access_token: &SecretString,
    ) -> Result<(), HubspotError> {
        let _ = tokio::try_join!(
            self.create_property_group(
                ObjectType::Companies,
                NewPropertyGroup {
                    name: PropertyGroup::MeteroidInfo.to_string(),
                    display_order: None,
                    label: "Meteroid information".to_string(),
                },
                access_token,
            ),
            self.create_property_group(
                ObjectType::Deals,
                NewPropertyGroup {
                    name: PropertyGroup::MeteroidInfo.to_string(),
                    display_order: None,
                    label: "Meteroid information".to_string(),
                },
                access_token,
            ),
        )?;

        let _ = tokio::try_join!(
            self.batch_create_properties(ObjectType::Companies, company_properties(), access_token),
            self.batch_create_properties(ObjectType::Deals, deal_properties(), access_token)
        )?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl PropertiesApi for HubspotClient {
    /// https://developers.hubspot.com/docs/reference/api/crm/properties#post-%2Fcrm%2Fv3%2Fproperties%2F%7Bobjecttype%7D%2Fbatch%2Fcreate
    async fn batch_create_properties(
        &self,
        object_type: ObjectType,
        properties: Vec<NewProperty>,
        access_token: &SecretString,
    ) -> Result<BatchCreateResponse, HubspotError> {
        self.execute(
            &format!("/crm/v3/properties/{object_type}/batch/create"),
            reqwest::Method::POST,
            access_token,
            Some(BatchActionRequest { inputs: properties }),
        )
        .await
    }

    /// https://developers.hubspot.com/docs/reference/api/crm/properties#post-%2Fcrm%2Fv3%2Fproperties%2F%7Bobjecttype%7D%2Fgroups
    async fn create_property_group(
        &self,
        object_type: ObjectType,
        property_group: NewPropertyGroup,
        access_token: &SecretString,
    ) -> Result<(), HubspotError> {
        let group_name = property_group.name.clone();
        let res: Result<serde_json::Value, HubspotError> = self
            .execute(
                &format!("/crm/v3/properties/{object_type}/groups"),
                reqwest::Method::POST,
                access_token,
                Some(property_group),
            )
            .await;

        match res {
            Ok(_) => Ok(()),
            Err(HubspotError::ClientError {
                status_code: Some(409),
                ..
            }) => {
                log::warn!("Property group {group_name} already exists");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

fn company_properties() -> Vec<NewProperty> {
    vec![
        NewProperty {
            name: CompanyProperty::MeteroidCustomerId.to_string(),
            description: Some("Customer ID in Meteroid".to_string()),
            label: "Meteroid customer ID".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: true,
            hidden: false,
        },
        NewProperty {
            name: CompanyProperty::MeteroidCustomerEmail.to_string(),
            description: Some("Customer billing email in Meteroid".to_string()),
            label: "Meteroid customer email".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: CompanyProperty::MeteroidCustomerCountry.to_string(),
            description: Some("Customer country in Meteroid".to_string()),
            label: "Meteroid customer country".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: CompanyProperty::MeteroidCustomerCity.to_string(),
            description: Some("Customer city in Meteroid".to_string()),
            label: "Meteroid customer city".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: CompanyProperty::MeteroidCustomerState.to_string(),
            description: Some("Customer state in Meteroid".to_string()),
            label: "Meteroid customer state".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: CompanyProperty::MeteroidCustomerStreet.to_string(),
            description: Some("Customer street in Meteroid".to_string()),
            label: "Meteroid customer street".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: CompanyProperty::MeteroidCustomerPostalCode.to_string(),
            description: Some("Customer postal code in Meteroid".to_string()),
            label: "Meteroid customer postal code".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
    ]
}

fn deal_properties() -> Vec<NewProperty> {
    vec![
        NewProperty {
            name: DealProperty::MeteroidCustomerId.to_string(),
            description: Some("Customer ID in Meteroid".to_string()),
            label: "Meteroid customer ID".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionId.to_string(),
            description: Some("Subscription ID in Meteroid".to_string()),
            label: "Meteroid subscription ID".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: true,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionPlan.to_string(),
            description: Some("Subscription plan in Meteroid".to_string()),
            label: "Meteroid subscription plan".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionStartDate.to_string(),
            description: Some("Subscription start date in Meteroid".to_string()),
            label: "Meteroid subscription start date".to_string(),
            type_: PropertyType::Date,
            field_type: PropertyFieldType::Date,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionEndDate.to_string(),
            description: Some("Subscription end date in Meteroid".to_string()),
            label: "Meteroid subscription end date".to_string(),
            type_: PropertyType::Date,
            field_type: PropertyFieldType::Date,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionCurrency.to_string(),
            description: Some("Subscription currency in Meteroid".to_string()),
            label: "Meteroid subscription currency".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionStatus.to_string(),
            description: Some("Subscription status in Meteroid".to_string()),
            label: "Meteroid subscription status".to_string(),
            type_: PropertyType::String,
            field_type: PropertyFieldType::Text,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
        NewProperty {
            name: DealProperty::MeteroidSubscriptionMrrCents.to_string(),
            description: Some("Subscription MRR (cents) in Meteroid".to_string()),
            label: "Meteroid subscription MRR (cents)".to_string(),
            type_: PropertyType::Number,
            field_type: PropertyFieldType::Number,
            group_name: PropertyGroup::MeteroidInfo.to_string(),
            has_unique_value: false,
            hidden: false,
        },
    ]
}

#[derive(strum::Display, Serialize, Deserialize)]
pub enum ObjectType {
    #[strum(to_string = "companies")]
    #[serde(rename = "companies")]
    Companies,
    #[strum(to_string = "deals")]
    #[serde(rename = "deals")]
    Deals,
}

#[derive(strum::Display, Serialize, Deserialize)]
pub enum PropertyType {
    #[strum(to_string = "string")]
    #[serde(rename = "string")]
    String,
    #[strum(to_string = "number")]
    #[serde(rename = "number")]
    Number,
    #[strum(to_string = "date")]
    #[serde(rename = "date")]
    Date,
}

#[derive(strum::Display, Serialize, Deserialize)]
pub enum PropertyFieldType {
    #[strum(to_string = "text")]
    #[serde(rename = "text")]
    Text,
    #[strum(to_string = "number")]
    #[serde(rename = "number")]
    Number,
    #[strum(to_string = "date")]
    #[serde(rename = "date")]
    Date,
}

#[derive(Serialize)]
pub struct NewProperty {
    pub name: String,
    pub description: Option<String>,
    pub label: String,
    #[serde(rename = "type")]
    pub type_: PropertyType,
    #[serde(rename = "fieldType")]
    pub field_type: PropertyFieldType,
    #[serde(rename = "groupName")]
    pub group_name: String,
    #[serde(rename = "hasUniqueValue")]
    pub has_unique_value: bool,
    pub hidden: bool,
}

#[derive(Debug, Deserialize)]
pub struct BatchCreateResponse {
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

#[derive(Serialize)]
pub struct NewPropertyGroup {
    name: String,
    #[serde(rename = "displayOrder")]
    display_order: Option<i32>,
    label: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct NewPropertyGroupResponse {
    name: String,
    label: String,
    #[serde(rename = "displayOrder")]
    display_order: Option<i32>,
    archived: bool,
}

#[derive(strum::Display)]
enum PropertyGroup {
    #[strum(to_string = "meteroidinfo")]
    MeteroidInfo,
}

#[derive(strum::Display)]
#[allow(clippy::enum_variant_names)]
enum CompanyProperty {
    #[strum(to_string = "meteroid_customer_id")]
    MeteroidCustomerId,
    #[strum(to_string = "meteroid_customer_email")]
    MeteroidCustomerEmail,
    #[strum(to_string = "meteroid_customer_country")]
    MeteroidCustomerCountry,
    #[strum(to_string = "meteroid_customer_city")]
    MeteroidCustomerCity,
    #[strum(to_string = "meteroid_customer_state")]
    MeteroidCustomerState,
    #[strum(to_string = "meteroid_customer_street")]
    MeteroidCustomerStreet,
    #[strum(to_string = "meteroid_customer_postal_code")]
    MeteroidCustomerPostalCode,
}

#[derive(strum::Display)]
#[allow(clippy::enum_variant_names)]
enum DealProperty {
    #[strum(to_string = "meteroid_customer_id")]
    MeteroidCustomerId,
    #[strum(to_string = "meteroid_subscription_id")]
    MeteroidSubscriptionId,
    #[strum(to_string = "meteroid_subscription_plan")]
    MeteroidSubscriptionPlan,
    #[strum(to_string = "meteroid_subscription_start_date")]
    MeteroidSubscriptionStartDate,
    #[strum(to_string = "meteroid_subscription_end_date")]
    MeteroidSubscriptionEndDate,
    #[strum(to_string = "meteroid_subscription_currency")]
    MeteroidSubscriptionCurrency,
    #[strum(to_string = "meteroid_subscription_status")]
    MeteroidSubscriptionStatus,
    #[strum(to_string = "meteroid_subscription_mrr_cents")]
    MeteroidSubscriptionMrrCents,
}
