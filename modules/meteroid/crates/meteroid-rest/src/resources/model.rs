use common_domain::ids::{CustomerId, ResourceId, ResourceTypeId, string_serde, string_serde_opt};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_rest_common::model::{PaginatedRequest, PaginationResponse};
use meteroid_store::domain::resources as domain;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use validator::Validate;

/// What a customer has several of (sites, workspaces), one level below the customer.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct ResourceType {
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub id: ResourceTypeId,
    /// Immutable: conditions name attributes as `<code>.<attribute>`.
    pub code: String,
    pub name: String,
    pub plural_name: String,
    /// Always null: every type sits directly below the customer.
    #[serde(with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub parent_type_id: Option<ResourceTypeId>,
    /// Identifying keys, the canonical one first, then its aliases. Unique across the tenant's
    /// types, archived ones included.
    pub keys: Vec<String>,
    /// Each attribute with its closed list of values.
    pub attributes: BTreeMap<String, Vec<String>>,
    /// Archived types still resolve in checks, but take no new resources or entitlement rows.
    pub archived: bool,
    pub created_at: ApiDateTime,
}

impl From<domain::ResourceType> for ResourceType {
    fn from(t: domain::ResourceType) -> Self {
        Self {
            id: t.id,
            code: t.code,
            name: t.name,
            plural_name: t.plural_name,
            parent_type_id: None,
            keys: t.keys,
            attributes: t.attributes,
            archived: t.archived_at.is_some(),
            created_at: t.created_at.into(),
        }
    }
}

#[derive(Debug, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ResourceTypeCreateRequest {
    /// Lowercase letters, digits and underscores, e.g. `site`. Immutable.
    #[validate(length(min = 1, max = 64))]
    pub code: String,
    #[validate(length(min = 1, max = 128))]
    pub name: String,
    #[validate(length(min = 1, max = 128))]
    pub plural_name: String,
    /// Refused with 400 `RESOURCE_HIERARCHY_UNSUPPORTED`: no second level yet.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub parent_type_id: Option<ResourceTypeId>,
    /// At least one, canonical first, e.g. `["site_id", "website_id"]`. A key another type holds
    /// gives 409 `RESOURCE_KEY_CLASH`.
    #[validate(length(min = 1))]
    pub keys: Vec<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, Vec<String>>,
}

/// Partial update: an absent field is left unchanged.
#[derive(Debug, Default, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ResourceTypeUpdateRequest {
    #[validate(length(min = 1, max = 128))]
    pub name: Option<String>,
    #[validate(length(min = 1, max = 128))]
    pub plural_name: Option<String>,
    /// Refused with 400 `RESOURCE_HIERARCHY_UNSUPPORTED`.
    #[serde(default, with = "string_serde_opt")]
    #[schema(value_type = Option<String>)]
    pub parent_type_id: Option<ResourceTypeId>,
    /// Replaces the keys, with the same clash check as on create.
    pub keys: Option<Vec<String>>,
    /// Replaces the attributes. Removing a value a resource or an entitlement condition uses is
    /// refused with 409, listing the usages.
    pub attributes: Option<BTreeMap<String, Vec<String>>>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, IntoParams, Deserialize, Validate)]
#[into_params(parameter_in = Query)]
pub struct ResourceTypeListRequest {
    /// Include archived types. Defaults to false.
    pub include_archived: Option<bool>,
}

#[derive(Debug, ToSchema, Serialize)]
pub struct ResourceTypeListResponse {
    pub data: Vec<ResourceType>,
}

/// A customer's stored resource: needed for one-resource overrides, subscriptions, attributes the
/// caller must not assert, and listing.
#[derive(Clone, Debug, ToSchema, Serialize)]
pub struct Resource {
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub id: ResourceId,
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub customer_id: CustomerId,
    #[serde(with = "string_serde")]
    #[schema(value_type = String)]
    pub resource_type_id: ResourceTypeId,
    /// The value of the type's keys, e.g. `s1`.
    pub key_value: String,
    pub name: Option<String>,
    /// `{"<attribute>": "<value>"}`, without the type code. They win over asserted ones in checks.
    pub attributes: BTreeMap<String, String>,
    pub archived: bool,
}

impl From<domain::Resource> for Resource {
    fn from(r: domain::Resource) -> Self {
        Self {
            id: r.id,
            customer_id: r.customer_id,
            resource_type_id: r.resource_type_id,
            key_value: r.key_value,
            name: r.name,
            attributes: r.attributes,
            archived: r.archived_at.is_some(),
        }
    }
}

/// Replaces the name, attributes and archive of the resource, creating it when new.
#[derive(Debug, Default, ToSchema, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct ResourceUpsertRequest {
    #[validate(length(min = 1, max = 256))]
    pub name: Option<String>,
    /// From the type's closed lists, without the type code.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// Archived resources still resolve in checks. Defaults to false.
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ResourceListRequest {
    #[serde(flatten)]
    #[validate(nested)]
    pub pagination: PaginatedRequest,
    /// The type's code or id.
    #[serde(rename = "type")]
    #[param(value_type = Option<String>)]
    pub resource_type: Option<common_domain::ids::AliasOr<ResourceTypeId>>,
    /// Include archived resources. Defaults to false.
    pub include_archived: Option<bool>,
}

#[derive(Debug, ToSchema, Serialize)]
pub struct ResourceListResponse {
    pub data: Vec<Resource>,
    pub pagination_meta: PaginationResponse,
}
