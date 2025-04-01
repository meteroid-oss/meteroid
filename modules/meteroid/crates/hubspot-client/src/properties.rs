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

    async fn init_meteroid_properties(
        &self,
        access_token: &SecretString,
    ) -> Result<(), HubspotError> {
        self.create_property_group(
            ObjectType::Companies,
            NewPropertyGroup {
                name: PropertyGroup::MeteroidInfo.to_string(),
                display_order: None,
                label: "Meteroid Info".to_string(),
            },
            access_token,
        )
        .await?;

        self.create_property_group(
            ObjectType::Deals,
            NewPropertyGroup {
                name: PropertyGroup::MeteroidInfo.to_string(),
                display_order: None,
                label: "Meteroid Info".to_string(),
            },
            access_token,
        )
        .await?;

        self.batch_create_properties(ObjectType::Companies, company_properties(), access_token)
            .await?;

        let deal_properties = vec![];

        self.batch_create_properties(ObjectType::Deals, deal_properties, access_token)
            .await?;
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
                log::warn!("Property group {} already exists", group_name);
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
}

#[derive(strum::Display, Serialize, Deserialize)]
pub enum PropertyFieldType {
    #[strum(to_string = "text")]
    #[serde(rename = "text")]
    Text,
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
    pub errors: Option<StandardErrorResponse>, // for status_207 responses (multiple statuses)
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
