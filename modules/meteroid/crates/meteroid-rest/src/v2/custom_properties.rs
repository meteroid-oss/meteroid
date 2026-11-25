//! `/api/v2/custom-property-definitions` (ADR-0013 §18): the properties a tenant defines on its
//! customers, subscriptions, invoices, credit notes and quotes, which those resources'
//! `custom_properties` are checked against. Core reads them, as v1 does; creating, changing and
//! archiving them is the Enterprise `custom-properties` module's (its `RestRoutes::v2`), which
//! uses the model and the checks here.
use crate::AppState;
use crate::v2::extract::{Path, Query};
use crate::v2::pagination::{PageQuery, page};
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{BaseId, CustomPropertyDefinitionId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::custom_property_definitions as domain;
use meteroid_store::domain::enums::{
    CustomPropertyEntityType as DomainEntity, CustomPropertyType as DomainType,
};
use meteroid_store::repositories::CustomPropertyDefinitionsInterface;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_definitions))
        .routes(routes!(get_definition))
}

// ── Model ───────────────────────────────────────────────────────────

/// The resource a definition applies to.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomPropertyEntity {
    Customer,
    Subscription,
    Invoice,
    CreditNote,
    Quote,
}

impl From<CustomPropertyEntity> for DomainEntity {
    fn from(entity: CustomPropertyEntity) -> Self {
        match entity {
            CustomPropertyEntity::Customer => DomainEntity::Customer,
            CustomPropertyEntity::Subscription => DomainEntity::Subscription,
            CustomPropertyEntity::Invoice => DomainEntity::Invoice,
            CustomPropertyEntity::CreditNote => DomainEntity::CreditNote,
            CustomPropertyEntity::Quote => DomainEntity::Quote,
        }
    }
}

impl From<DomainEntity> for CustomPropertyEntity {
    fn from(entity: DomainEntity) -> Self {
        match entity {
            DomainEntity::Customer => CustomPropertyEntity::Customer,
            DomainEntity::Subscription => CustomPropertyEntity::Subscription,
            DomainEntity::Invoice => CustomPropertyEntity::Invoice,
            DomainEntity::CreditNote => CustomPropertyEntity::CreditNote,
            DomainEntity::Quote => CustomPropertyEntity::Quote,
        }
    }
}

/// What a property's value is.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomPropertyType {
    Text,
    Number,
    Boolean,
    /// `YYYY-MM-DD`.
    Date,
    /// RFC 3339.
    Datetime,
    /// One of `config.options`.
    SingleSelect,
    /// Some of `config.options`.
    MultiSelect,
    Json,
    Url,
    Email,
}

impl From<CustomPropertyType> for DomainType {
    fn from(kind: CustomPropertyType) -> Self {
        match kind {
            CustomPropertyType::Text => DomainType::Text,
            CustomPropertyType::Number => DomainType::Number,
            CustomPropertyType::Boolean => DomainType::Boolean,
            CustomPropertyType::Date => DomainType::Date,
            CustomPropertyType::Datetime => DomainType::Datetime,
            CustomPropertyType::SingleSelect => DomainType::SingleSelect,
            CustomPropertyType::MultiSelect => DomainType::MultiSelect,
            CustomPropertyType::Json => DomainType::Json,
            CustomPropertyType::Url => DomainType::Url,
            CustomPropertyType::Email => DomainType::Email,
        }
    }
}

impl From<DomainType> for CustomPropertyType {
    fn from(kind: DomainType) -> Self {
        match kind {
            DomainType::Text => CustomPropertyType::Text,
            DomainType::Number => CustomPropertyType::Number,
            DomainType::Boolean => CustomPropertyType::Boolean,
            DomainType::Date => CustomPropertyType::Date,
            DomainType::Datetime => CustomPropertyType::Datetime,
            DomainType::SingleSelect => CustomPropertyType::SingleSelect,
            DomainType::MultiSelect => CustomPropertyType::MultiSelect,
            DomainType::Json => CustomPropertyType::Json,
            DomainType::Url => CustomPropertyType::Url,
            DomainType::Email => CustomPropertyType::Email,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomPropertyOption {
    pub value: String,
    /// Shown in place of the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// What a value is checked against; only the fields of the property's type are read.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct CustomPropertyConfig {
    /// The choices of a `single_select` or `multi_select`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<CustomPropertyOption>>,
    /// The bounds of a `number`, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// The longest a `text` may be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}

impl From<CustomPropertyConfig> for domain::PropertyConfig {
    fn from(config: CustomPropertyConfig) -> Self {
        domain::PropertyConfig {
            options: config.options.map(|options| {
                options
                    .into_iter()
                    .map(|option| domain::SelectOption {
                        value: option.value,
                        label: option.label,
                    })
                    .collect()
            }),
            min: config.min,
            max: config.max,
            max_length: config.max_length,
        }
    }
}

impl From<domain::PropertyConfig> for CustomPropertyConfig {
    fn from(config: domain::PropertyConfig) -> Self {
        CustomPropertyConfig {
            options: config.options.map(|options| {
                options
                    .into_iter()
                    .map(|option| CustomPropertyOption {
                        value: option.value,
                        label: option.label,
                    })
                    .collect()
            }),
            min: config.min,
            max: config.max,
            max_length: config.max_length,
        }
    }
}

/// A custom property: the key a resource's `custom_properties` holds its value under, and what
/// that value may be.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CustomPropertyDefinition {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CustomPropertyDefinitionId,
    pub entity_type: CustomPropertyEntity,
    /// Unique among the live definitions of its entity type, immutable.
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Immutable.
    #[serde(rename = "type")]
    pub property_type: CustomPropertyType,
    pub config: CustomPropertyConfig,
    /// A resource of its type is refused without a value.
    pub required: bool,
    /// The value a resource created without one gets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_json::Value>,
    /// Where the dashboard shows it, lowest first.
    pub display_order: i32,
    /// Archived: no longer checked on writes; the values held are kept.
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<ApiDateTime>,
    pub created_at: ApiDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<ApiDateTime>,
}

crate::v2_list!(CustomPropertyDefinitionList, CustomPropertyDefinition);

pub fn to_api(definition: domain::CustomPropertyDefinition) -> CustomPropertyDefinition {
    CustomPropertyDefinition {
        id: definition.id,
        entity_type: definition.entity_type.into(),
        key: definition.key,
        name: definition.name,
        description: definition.description,
        property_type: definition.property_type.into(),
        config: definition.config.into(),
        required: definition.required,
        default_value: definition.default_value,
        display_order: definition.display_order,
        archived: definition.archived_at.is_some(),
        archived_at: definition.archived_at.map(Into::into),
        created_at: definition.created_at.into(),
        updated_at: definition.updated_at.map(Into::into),
    }
}

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct CustomPropertyDefinitionListQuery {
    /// Only the definitions of this entity type.
    pub entity_type: Option<CustomPropertyEntity>,
    /// Archived definitions too.
    #[serde(default)]
    pub include_archived: bool,
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateCustomPropertyDefinition {
    pub entity_type: CustomPropertyEntity,
    /// Letters, digits and `_`, at most 128; immutable.
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub property_type: CustomPropertyType,
    #[serde(default)]
    pub config: CustomPropertyConfig,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default)]
    pub display_order: i32,
}

/// A merge patch: a field left out is unchanged, `null` clears it. The key, the entity type and
/// the type are fixed.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateCustomPropertyDefinition {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Replaces the config whole.
    #[serde(default)]
    pub config: Option<CustomPropertyConfig>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<Value>)]
    pub default_value: Option<Option<serde_json::Value>>,
    #[serde(default)]
    pub display_order: Option<i32>,
}

// ── Checks, shared with the module's writes ─────────────────────────

const KEY_MAX: usize = 128;

fn check_name(name: &str) -> Result<(), Problem> {
    if name.trim().is_empty() {
        return Err(Problem::field("/name", "required", "A name is not blank"));
    }
    Ok(())
}

/// A create checked at its pointers before the store sees it.
pub fn check_create(input: &CreateCustomPropertyDefinition) -> Result<(), Problem> {
    let key = input.key.as_str();
    if key.is_empty()
        || key.len() > KEY_MAX
        || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(Problem::field(
            "/key",
            "invalid",
            format!("A key is 1 to {KEY_MAX} letters, digits and underscores"),
        ));
    }
    check_name(&input.name)
}

/// A patch checked at its pointers before the store sees it.
pub fn check_update(input: &UpdateCustomPropertyDefinition) -> Result<(), Problem> {
    match input.name.as_deref() {
        Some(name) => check_name(name),
        None => Ok(()),
    }
}

/// Another live definition of the entity type holding the key (a create, or an unarchive).
pub fn key_taken(problem: Problem, key: &str) -> Problem {
    if problem.code != ProblemCode::AlreadyExists {
        return problem;
    }
    let message = format!("A live definition holds the key '{key}'");
    let mut taken = Problem::conflict(ProblemCode::CodeTaken, message.clone());
    taken.errors.push(FieldError {
        path: "/key".to_owned(),
        code: "duplicate".to_owned(),
        message,
    });
    taken
}

/// The definition a path names, by id (`cpd_…` or its UUID); its tenant is checked by the read.
pub fn definition_id(raw: &str) -> Result<CustomPropertyDefinitionId, Problem> {
    let raw = raw.trim();
    CustomPropertyDefinitionId::parse_uuid(raw)
        .or_else(|_| CustomPropertyDefinitionId::from_str(raw))
        .map_err(|_| Problem::not_found(format!("custom property definition {raw}")))
}

// ── Reads ───────────────────────────────────────────────────────────

/// List custom property definitions
///
/// The tenant's definitions in creation order, paged by cursor; archived ones left out unless
/// asked.
#[utoipa::path(
    get,
    tag = "Custom properties",
    path = "/api/v2/custom-property-definitions",
    operation_id = "list_custom_property_definitions",
    params(CustomPropertyDefinitionListQuery),
    responses(
        (status = 200, description = "A page of definitions", body = CustomPropertyDefinitionList),
        (status = 422, description = "A filter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_definitions(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<CustomPropertyDefinitionListQuery>,
) -> Result<Json<CustomPropertyDefinitionList>, Problem> {
    let definitions = state
        .store
        .list_custom_property_definitions(
            auth.tenant_id,
            query.entity_type.map(Into::into),
            query.include_archived,
        )
        .await?;
    let paged = page(
        definitions,
        |definition| definition.id.as_uuid(),
        &PageQuery {
            limit: query.limit,
            after: query.after,
        },
    )?;
    Ok(Json(paged.map(to_api).into()))
}

/// Get a custom property definition
///
/// By id; an archived one too.
#[utoipa::path(
    get,
    tag = "Custom properties",
    path = "/api/v2/custom-property-definitions/{id}",
    operation_id = "get_custom_property_definition",
    params(("id" = String, Path, description = "Definition id")),
    responses(
        (status = 200, description = "The definition", body = CustomPropertyDefinition),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_definition(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CustomPropertyDefinition>, Problem> {
    let id = definition_id(&id)?;
    let definition = state
        .store
        .get_custom_property_definition(auth.tenant_id, id)
        .await?;
    Ok(Json(to_api(definition)))
}
