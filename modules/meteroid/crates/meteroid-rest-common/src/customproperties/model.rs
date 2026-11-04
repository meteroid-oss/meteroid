use crate::model::{PaginatedRequest, PaginationResponse};
use common_domain::ids::{CustomPropertyDefinitionId, string_serde};
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomPropertyEntityType {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ToSchema, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CustomPropertyType {
    Text,
    Number,
    Boolean,
    Date,
    Datetime,
    SingleSelect,
    MultiSelect,
    Json,
    Url,
    Email,
}

#[derive(Clone, Debug, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: Option<String>,
}

/// Type-specific configuration. Only the fields relevant to `property_type` are interpreted.
#[derive(Clone, Debug, Default, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct PropertyConfig {
    /// Allowed choices for `SINGLE_SELECT` / `MULTI_SELECT`.
    pub options: Option<Vec<SelectOption>>,
    /// Inclusive numeric bounds for `NUMBER`.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Maximum length for `TEXT`.
    pub max_length: Option<u32>,
}

#[derive(Clone, ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomPropertyDefinition {
    #[serde(with = "string_serde")]
    pub id: CustomPropertyDefinitionId,
    pub entity_type: CustomPropertyEntityType,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub property_type: CustomPropertyType,
    pub config: PropertyConfig,
    pub required: bool,
    #[schema(value_type = Option<serde_json::Value>)]
    pub default_value: Option<serde_json::Value>,
    pub display_order: i32,
    pub archived: bool,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate)]
pub struct CustomPropertyDefinitionCreateRequest {
    pub entity_type: CustomPropertyEntityType,
    /// Immutable machine name; letters, digits and underscores only. Unique per entity type.
    #[validate(length(min = 1, max = 128))]
    pub key: String,
    #[validate(length(min = 1))]
    pub name: String,
    pub description: Option<String>,
    pub property_type: CustomPropertyType,
    #[serde(default)]
    pub config: PropertyConfig,
    #[serde(default)]
    pub required: bool,
    #[schema(value_type = Option<serde_json::Value>)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub display_order: i32,
}

/// Update of a definition. `key`, `entity_type` and `property_type` are immutable and cannot be
/// changed here. Any field left absent is unchanged.
#[derive(ToSchema, serde::Serialize, serde::Deserialize, Validate, Default)]
pub struct CustomPropertyDefinitionUpdateRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub config: Option<PropertyConfig>,
    pub required: Option<bool>,
    #[schema(value_type = Option<Option<serde_json::Value>>)]
    pub default_value: Option<Option<serde_json::Value>>,
    pub display_order: Option<i32>,
}

#[derive(ToSchema, IntoParams, serde::Serialize, serde::Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct CustomPropertyDefinitionListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// Filter to a single entity type.
    pub entity_type: Option<CustomPropertyEntityType>,
    /// Include archived (soft-deleted) definitions. Defaults to false.
    pub include_archived: Option<bool>,
}

#[derive(ToSchema, serde::Serialize, serde::Deserialize)]
pub struct CustomPropertyDefinitionListResponse {
    pub data: Vec<CustomPropertyDefinition>,
    pub pagination_meta: PaginationResponse,
}
