//! `/api/v2/features`: what a customer may use (a flag, a usage cap, a setting), granted by
//! plan versions, add-ons and products (`v2::entitlements`). A feature's code is what an
//! application checks; archiving one frees its code and hides its grants until it is
//! unarchived.
use crate::AppState;
use crate::v2::entitlements::{GrantValue, feature, grant_value, parse_feature, to_domain};
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use crate::v2::refs::{ProductRef, ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::Response;
use common_domain::ids::{AliasOr, BaseId, BillableMetricId, FeatureId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlements::{
    self as domain, EntitlementSpec, FeatureNew, FeatureUpdate,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, FeaturePageFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    Active,
    /// Switched off for every customer, its grants kept.
    Disabled,
    /// Hidden with its grants until unarchived; its code is free for another feature.
    Archived,
}

impl From<FeatureStatusEnum> for FeatureStatus {
    fn from(status: FeatureStatusEnum) -> Self {
        match status {
            FeatureStatusEnum::Active => FeatureStatus::Active,
            FeatureStatusEnum::Disabled => FeatureStatus::Disabled,
            FeatureStatusEnum::Archived => FeatureStatus::Archived,
        }
    }
}

impl From<FeatureStatus> for FeatureStatusEnum {
    fn from(status: FeatureStatus) -> Self {
        match status {
            FeatureStatus::Active => FeatureStatusEnum::Active,
            FeatureStatus::Disabled => FeatureStatusEnum::Disabled,
            FeatureStatus::Archived => FeatureStatusEnum::Archived,
        }
    }
}

/// What a feature is; fixed once created.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeatureKind {
    Boolean(BooleanFeature),
    Metered(MeteredFeature),
    Config(ConfigFeature),
}

/// On or off.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BooleanFeature {}

/// A cap on a metric's usage.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeteredFeature {
    /// The metric capped, by code (id or code on input).
    pub metric: String,
}

/// A typed setting.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfigFeature {
    pub value_type: ConfigValueKind,
    /// The values a `select` takes; required on one, refused on the others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValueKind {
    Number,
    Boolean,
    Text,
    Map,
    Json,
    Select,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Feature {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: FeatureId,
    /// What an application checks; unique among live features, immutable.
    pub code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub kind: FeatureKind,
    /// The product it belongs to; none: every product's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub status: FeatureStatus,
    /// What every customer gets unless a plan, an add-on or the subscription grants otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<GrantValue>,
    pub created_at: ApiDateTime,
}

crate::v2_list!(FeatureList, Feature);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct FeatureListQuery {
    /// Only the features of this product, by code or id.
    pub product: Option<String>,
    /// Only the features of this status; archived ones are left out by default.
    pub status: Option<FeatureStatus>,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateFeature {
    /// Lowercase letters, digits, `-` and `_`, starting with a letter or a digit; at most 128.
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub kind: FeatureKind,
    /// The product it belongs to, by code or id; none: every product's.
    #[serde(default)]
    pub product: Option<String>,
    /// What every customer gets unless granted otherwise.
    #[serde(default)]
    pub default: Option<GrantValue>,
}

/// A merge patch; the code, the type and the product never change.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateFeature {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// A `select`'s values; one still granted cannot be removed.
    #[serde(default)]
    pub options: Option<Vec<String>>,
    /// `null` removes the default.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<GrantValue>)]
    pub default: Option<Option<GrantValue>>,
}

const MAX_CODE_LENGTH: usize = 128;

fn check_code(code: &str) -> Result<(), Problem> {
    let valid = !code.is_empty()
        && code.len() <= MAX_CODE_LENGTH
        && code
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !code.starts_with(FeatureId::PREFIX);
    if valid {
        Ok(())
    } else {
        Err(Problem::field(
            "/code",
            "invalid",
            format!(
                "A feature code is 1 to {MAX_CODE_LENGTH} lowercase letters, digits, `-` and `_`, \
                 starting with a letter or a digit, and never with `{}`",
                FeatureId::PREFIX
            ),
        ))
    }
}

/// The stored type of `kind`, its metric resolved and checked to be the tenant's.
async fn feature_type(
    state: &AppState,
    tenant_id: TenantId,
    kind: &FeatureKind,
) -> Result<domain::FeatureType, Problem> {
    Ok(match kind {
        FeatureKind::Boolean(_) => domain::FeatureType::Boolean,
        FeatureKind::Metered(metered) => {
            let metric = ids::metric(state, tenant_id, &metered.metric)
                .await
                .map_err(|problem| {
                    if problem.status == 404 {
                        Problem::field(
                            "/type/metric",
                            "not_found",
                            format!("metric {} not found", metered.metric),
                        )
                    } else {
                        problem
                    }
                })?;
            domain::FeatureType::Metered {
                metric_id: metric.id,
            }
        }
        FeatureKind::Config(config) => {
            let select = config.value_type == ConfigValueKind::Select;
            if select && config.options.is_empty() {
                return Err(Problem::field(
                    "/type/options",
                    "required",
                    "A `select` takes one value at least",
                ));
            }
            if !select && !config.options.is_empty() {
                return Err(Problem::field(
                    "/type/options",
                    "invalid",
                    "Only a `select` has options",
                ));
            }
            domain::FeatureType::Config {
                value_type: match config.value_type {
                    ConfigValueKind::Number => domain::ConfigValueType::Number,
                    ConfigValueKind::Boolean => domain::ConfigValueType::Boolean,
                    ConfigValueKind::Text => domain::ConfigValueType::Text,
                    ConfigValueKind::Map => domain::ConfigValueType::Map,
                    ConfigValueKind::Json => domain::ConfigValueType::Json,
                    ConfigValueKind::Select => domain::ConfigValueType::Select {
                        options: config.options.clone(),
                    },
                },
            }
        }
    })
}

fn kind(
    feature_type: &domain::FeatureType,
    metrics: &HashMap<BillableMetricId, String>,
) -> FeatureKind {
    match feature_type {
        domain::FeatureType::Boolean => FeatureKind::Boolean(BooleanFeature {}),
        domain::FeatureType::Metered { metric_id } => FeatureKind::Metered(MeteredFeature {
            metric: metrics
                .get(metric_id)
                .cloned()
                .unwrap_or_else(|| metric_id.to_string()),
        }),
        domain::FeatureType::Config { value_type } => {
            let (value_type, options) = match value_type {
                domain::ConfigValueType::Number => (ConfigValueKind::Number, Vec::new()),
                domain::ConfigValueType::Boolean => (ConfigValueKind::Boolean, Vec::new()),
                domain::ConfigValueType::Text => (ConfigValueKind::Text, Vec::new()),
                domain::ConfigValueType::Map => (ConfigValueKind::Map, Vec::new()),
                domain::ConfigValueType::Json => (ConfigValueKind::Json, Vec::new()),
                domain::ConfigValueType::Select { options } => {
                    (ConfigValueKind::Select, options.clone())
                }
            };
            FeatureKind::Config(ConfigFeature {
                value_type,
                options,
            })
        }
    }
}

async fn to_api(
    state: &AppState,
    tenant_id: TenantId,
    features: Vec<domain::Feature>,
) -> Result<Vec<Feature>, Problem> {
    let mut metric_ids: Vec<BillableMetricId> = features
        .iter()
        .filter_map(|feature| match feature.feature_type {
            domain::FeatureType::Metered { metric_id } => Some(metric_id),
            _ => None,
        })
        .collect();
    metric_ids.sort_by_key(|id| id.as_uuid());
    metric_ids.dedup();
    let metrics: HashMap<BillableMetricId, String> = state
        .store
        .find_billable_metrics_by_ids(&metric_ids, tenant_id)
        .await?
        .into_iter()
        .map(|metric| (metric.id, metric.catalog_code))
        .collect();
    let ids = ReferenceIds {
        products: features
            .iter()
            .filter_map(|feature| feature.product.as_ref().map(|product| product.id))
            .collect(),
        ..ReferenceIds::default()
    };
    let refs = Refs::load(&state.store, tenant_id, &ids).await?;
    Ok(features
        .into_iter()
        .map(|feature| Feature {
            id: feature.id,
            kind: kind(&feature.feature_type, &metrics),
            product: feature.product.map(|product| refs.product(product.id)),
            status: feature.status.into(),
            default: feature
                .entitlement
                .as_ref()
                .map(|entitlement| grant_value(&entitlement.value)),
            created_at: feature.created_at.into(),
            code: feature.code,
            name: feature.name,
            description: feature.description,
        })
        .collect())
}

async fn one(
    state: &AppState,
    tenant_id: TenantId,
    feature: domain::Feature,
) -> Result<Feature, Problem> {
    to_api(state, tenant_id, vec![feature])
        .await?
        .pop()
        .ok_or_else(Problem::internal)
}

/// The feature written, as it reads; by its id, code, name and status when its metric and
/// product are not read back (a write committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    feature: domain::Feature,
) -> Response {
    let known = serde_json::json!({
        "id": feature.id.to_string(),
        "code": feature.code.clone(),
        "name": feature.name.clone(),
        "status": FeatureStatus::from(feature.status.clone()),
    });
    committed_write(status, one(state, tenant_id, feature).await, || known)
}

/// The feature read again once its status is set (the store answers nothing); by its id and
/// status when it is not read back.
async fn reread(
    state: &AppState,
    tenant_id: TenantId,
    id: FeatureId,
    status: FeatureStatus,
) -> Response {
    let read = match state.store.get_feature(id, tenant_id).await {
        Ok(feature) => one(state, tenant_id, feature).await,
        Err(error) => Err(error.into()),
    };
    committed_write(
        StatusCode::OK,
        read,
        || serde_json::json!({"id": id.to_string(), "status": status}),
    )
}

/// List features
#[utoipa::path(
    get,
    tag = "Features",
    path = "/api/v2/features",
    params(FeatureListQuery),
    responses(
        (status = 200, description = "A page of the tenant's features, archived ones left out unless asked", body = FeatureList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_features(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<FeatureListQuery>,
) -> Result<Json<FeatureList>, Problem> {
    let product_id = match query.product.as_deref() {
        Some(product) => Some(ids::product_id(&state, auth.tenant_id, product).await?),
        None => None,
    };
    let page = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page.limit()?;
    let after = page.cursor()?.map(|Cursor(after)| FeatureId::from(after));
    let statuses = match query.status {
        Some(status) => vec![status.into()],
        None => vec![FeatureStatusEnum::Active, FeatureStatusEnum::Disabled],
    };
    let features = state
        .store
        .list_features_after(
            auth.tenant_id,
            FeaturePageFilter {
                statuses: Some(statuses),
                product_id,
            },
            after,
            limit + 1,
        )
        .await?;
    let paged = keyset(features, limit, |feature| feature.id.as_uuid());
    let data = to_api(&state, auth.tenant_id, paged.data).await?;
    Ok(Json(FeatureList {
        data,
        next_cursor: paged.next_cursor,
        has_more: paged.has_more,
    }))
}

/// Get a feature
///
/// By id, or by the code of its live feature.
#[utoipa::path(
    get,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "The feature", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<Feature>, Problem> {
    let found = feature(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(one(&state, auth.tenant_id, found).await?))
}

/// Create a feature
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features",
    request_body = CreateFeature,
    responses(
        (status = 201, description = "Created", body = Feature),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateFeature>,
) -> Result<Response, Problem> {
    let code = body.code.trim().to_owned();
    check_code(&code)?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "A feature has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    let product_id = match body.product.as_deref() {
        Some(product) => Some(
            ids::product_id(&state, auth.tenant_id, product)
                .await
                .map_err(|problem| {
                    if problem.status == 404 {
                        Problem::field(
                            "/product",
                            "not_found",
                            format!("product {product} not found"),
                        )
                    } else {
                        problem
                    }
                })?,
        ),
        None => None,
    };
    let feature_type = feature_type(&state, auth.tenant_id, &body.kind).await?;
    let entitlement = match &body.default {
        Some(default) => Some(to_domain(default, &feature_type, "/default")?),
        None => None,
    };
    let created = state
        .store
        .create_feature(
            auth.as_actor(),
            FeatureNew {
                tenant_id: auth.tenant_id,
                product_id,
                name,
                code: code.clone(),
                description: body.description,
                feature_type,
                entitlement,
            },
        )
        .await
        .map_err(|error| Problem::at("/default", error).code_taken(&code))?;
    Ok(written(&state, auth.tenant_id, StatusCode::CREATED, created).await)
}

/// Update a feature
///
/// A JSON merge patch of its name, description, a `select`'s options and its default; the code,
/// the type and the product never change. An archived feature is unarchived first.
#[utoipa::path(
    patch,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    request_body(content = UpdateFeature, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateFeature>,
) -> Result<Response, Problem> {
    let current = feature(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "A feature has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    crate::v2::bounds::description(
        body.description.as_ref().and_then(|text| text.as_deref()),
        "/description",
    )?;
    if current.status == FeatureStatusEnum::Archived {
        return Err(Problem::conflict(
            crate::v2::problem::ProblemCode::Conflict,
            "The feature is archived: unarchive it first",
        ));
    }
    // Every field is checked before the first write.
    let default: Option<Vec<EntitlementSpec>> = match &body.default {
        None => None,
        Some(None) => Some(Vec::new()),
        Some(Some(value)) => Some(vec![EntitlementSpec {
            feature_id: current.id,
            value: to_domain(value, &current.feature_type, "/default")?,
        }]),
    };
    if body.options.is_some()
        && !matches!(
            current.feature_type,
            domain::FeatureType::Config {
                value_type: domain::ConfigValueType::Select { .. }
            }
        )
    {
        return Err(Problem::field(
            "/options",
            "invalid",
            "Only a `select` has options",
        ));
    }
    // Its fields and its default in one transaction: a refusal of either writes neither.
    let path = match (body.options.is_some(), default.is_some()) {
        (true, false) => Some("/options"),
        (false, true) => Some("/default"),
        _ => None,
    };
    let update = (body.name.is_some() || body.description.is_some() || body.options.is_some())
        .then(|| FeatureUpdate {
            name: body.name.map(|name| name.trim().to_owned()),
            description: body.description,
            product_id: None,
            config_options: body.options,
        });
    let updated = state
        .store
        .update_feature_with_default(current.id, auth.tenant_id, update, default)
        .await
        .map_err(|error| match path {
            Some(path) => Problem::at(path, error),
            None => Problem::from(error),
        })?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, updated).await)
}

/// Archive a feature
///
/// Turns it off and hides it and its grants until it is unarchived; its code is free for a new
/// feature.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "Archived", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let found = feature(&state, auth.tenant_id, &id_or_code).await?;
    state
        .store
        .set_feature_status(
            auth.as_actor(),
            found.id,
            auth.tenant_id,
            FeatureStatusEnum::Archived,
        )
        .await?;
    Ok(reread(&state, auth.tenant_id, found.id, FeatureStatus::Archived).await)
}

/// Unarchive a feature
///
/// Active again with all its grants. By id, or by the code it had when no live feature holds it.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "Unarchived", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Another feature holds its code", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let id = match parse_feature(&id_or_code) {
        AliasOr::Id(id) => id,
        AliasOr::Alias(code) => {
            state
                .store
                .find_archived_feature_by_code(&code, auth.tenant_id)
                .await?
        }
    };
    // Scoped by tenant: another tenant's id reads as not found.
    state
        .store
        .set_feature_status(
            auth.as_actor(),
            id,
            auth.tenant_id,
            FeatureStatusEnum::Active,
        )
        .await?;
    Ok(reread(&state, auth.tenant_id, id, FeatureStatus::Active).await)
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_features, create_feature))
        .routes(routes!(get_feature, update_feature))
        .routes(routes!(archive_feature))
        .routes(routes!(unarchive_feature))
}
