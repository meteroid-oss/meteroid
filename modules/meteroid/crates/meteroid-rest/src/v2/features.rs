//! `/api/v2/features` (ADR-0013 §23): what a customer may use (a flag, a value, a limit), its
//! policy, its unit, the level it applies per and its default rows. A feature's code is what an
//! application checks; archiving one frees its code and hides its rows until it is unarchived.
use crate::AppState;
use crate::v2::entitlements::model::{
    Codes, EntitlementInput, FeatureKey, FeatureKind, Policy, checked_policy,
};
use crate::v2::entitlements::{Entitlement, canonical, own_defaults};
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use crate::v2::refs::{ProductRef, ResourceTypeRef};
use axum::Extension;
use axum::Json;
use axum::extract::{RawQuery, State};
use axum::response::{IntoResponse, Response};
use common_domain::ids::{
    AliasOr, BaseId, BillableMetricId, FeatureId, ManagedQuantityId, ProductId, ResourceTypeId,
    TenantId,
};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlement_policy::FeaturePolicy;
use meteroid_store::domain::entitlements::{
    self as domain, FeatureKind as DomainKind, FeatureNew, FeatureProductRef, FeatureUpdate,
    ValueType,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::ManagedQuantitiesInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, FeaturePageFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

// ── Model ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    Active,
    /// Entitles no one; its rows and code stay.
    Disabled,
    /// Hidden with its rows until unarchived; its code is free for another feature.
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

/// What a feature is. Its type, `value_type`, `counted_by` and `metric` are fixed once created.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeatureType {
    Boolean(BooleanFeature),
    Value(ValueFeature),
    Limit(LimitFeature),
}

/// On or off.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BooleanFeature {}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureValueType {
    Number,
    Text,
    /// One of `options`.
    Select,
    Json,
}

/// A value: a number, a text, one of a select's options, or any JSON.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ValueFeature {
    pub value_type: FeatureValueType,
    /// A number's direction: a higher one is the better grant (true by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub higher_is_better: Option<bool>,
    /// A select's options, distinct; one a row still holds cannot be removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// A select's options rank its grants, the last the best (false by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordered: Option<bool>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LimitCountedBy {
    /// A metric's usage events.
    Events,
    /// Your app reports the count (`POST …/entitlements/{feature}/usage`).
    App,
}

/// A limit on usage.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LimitFeature {
    pub counted_by: LimitCountedBy,
    /// `events` only: the metric counted, by code (id or code on input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// A managed quantity, by code (id or code on input): the limit is then per unit a
    /// subscription holds ("5 projects per seat"), as an add-on's rows count per instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_unit_of: Option<String>,
    /// Always read; on create, left out: `{resets: billing_cycle, at_limit: deny}` from events,
    /// `{resets: never, at_limit: deny_new}` by the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
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
    /// The product it belongs to; none: every product's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub status: FeatureStatus,
    #[serde(rename = "type")]
    pub kind: FeatureType,
    /// What a number or a limit counts (`projects`, `requests per minute`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The resource type it applies per (each site has its own value); none: the customer as a
    /// whole. A customer is entitled only where a row gives a value at this level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_per: Option<ResourceTypeRef>,
    /// The feature's default rows: what every customer gets before any plan applies.
    pub defaults: Vec<Entitlement>,
    pub created_at: ApiDateTime,
    pub updated_at: ApiDateTime,
}

crate::v2_list!(FeatureList, Feature);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct FeatureListQuery {
    /// Only the features of this product, by code or id.
    pub product: Option<String>,
    /// Only the features of these statuses (repeatable); archived ones are left out unless
    /// asked.
    #[param(style = Form, explode)]
    pub status: Option<Vec<FeatureStatus>>,
    /// Only the features applying per this resource type (by code or id), or `customer`.
    pub applies_per: Option<String>,
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
    /// The product it belongs to, by code or id; none: every product's.
    #[serde(default)]
    pub product: Option<String>,
    #[serde(rename = "type")]
    pub kind: FeatureType,
    /// A number's or a limit's.
    #[serde(default)]
    pub unit: Option<String>,
    /// A resource type, by code or id, for a value per resource (per site); none: the customer
    /// as a whole.
    #[serde(default)]
    pub applies_per: Option<String>,
    /// The default rows, each naming this feature (its code): what every customer gets before
    /// any plan applies.
    #[serde(default)]
    pub defaults: Vec<EntitlementInput>,
}

/// A merge patch of the type's fields that change; the others, sent, must read as they are.
#[derive(Deserialize, ToSchema, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateFeatureType {
    /// Fixed.
    #[serde(default, rename = "type")]
    pub kind: Option<FeatureKind>,
    /// Fixed.
    #[serde(default)]
    pub value_type: Option<FeatureValueType>,
    #[serde(default)]
    pub higher_is_better: Option<bool>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub ordered: Option<bool>,
    /// Fixed.
    #[serde(default)]
    pub counted_by: Option<LimitCountedBy>,
    /// Fixed.
    #[serde(default)]
    pub metric: Option<String>,
    /// `null`: per subscription again.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub per_unit_of: Option<Option<String>>,
    #[serde(default)]
    pub policy: Option<Policy>,
}

/// A merge patch of the feature as written. The code never changes.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateFeature {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// `null`: every product's.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub product: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub unit: Option<Option<String>>,
    /// `null`: the customer. Refused while the feature has rows, or counts reported by your app.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub applies_per: Option<Option<String>>,
    #[serde(default, rename = "type")]
    pub kind: Option<UpdateFeatureType>,
    /// Replaces the default rows; `[]` removes them.
    #[serde(default)]
    pub defaults: Option<Vec<EntitlementInput>>,
}

// ── Mapping ─────────────────────────────────────────────────────────

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

fn fixed(path: &str, what: &str) -> Problem {
    Problem::field(
        path,
        "invalid",
        format!("A feature's {what} is fixed once created"),
    )
}

/// A 404 of `lookup` as a refusal of the field at `path`.
fn not_found_at(path: &str, what: &str, raw: &str) -> impl FnOnce(Problem) -> Problem {
    let (path, message) = (path.to_owned(), format!("No {what} '{}'", raw.trim()));
    move |problem| {
        if problem.status == 404 {
            Problem::field(path, "not_found", message)
        } else {
            problem
        }
    }
}

async fn product_of(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ProductId, Problem> {
    ids::product_id(state, tenant_id, raw)
        .await
        .map_err(not_found_at("/product", "product", raw))
}

/// A live resource type of an id or a code, at `path`.
async fn level_of(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
    path: &str,
) -> Result<ResourceTypeId, Problem> {
    let resource_type = ids::resource_type(state, tenant_id, raw)
        .await
        .map_err(not_found_at(path, "resource type", raw))?;
    if resource_type.archived_at.is_some() {
        return Err(Problem::field(
            path,
            "archived",
            format!("The resource type '{}' is archived", resource_type.code),
        ));
    }
    Ok(resource_type.id)
}

async fn quantity_of(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<ManagedQuantityId, Problem> {
    let path = "/type/per_unit_of";
    let quantity = ids::quantity(state, tenant_id, raw)
        .await
        .map_err(not_found_at(path, "managed quantity", raw))?;
    if quantity.archived_at.is_some() {
        return Err(Problem::field(
            path,
            "archived",
            format!("The managed quantity '{}' is archived", quantity.code),
        ));
    }
    Ok(quantity.id)
}

/// A unit, trimmed; only a number's or a limit's.
fn unit_of(unit: Option<&str>, kind: &DomainKind) -> Result<Option<String>, Problem> {
    let Some(unit) = unit.map(str::trim).filter(|unit| !unit.is_empty()) else {
        return Ok(None);
    };
    if !matches!(
        kind,
        DomainKind::Limit(_) | DomainKind::Value(ValueType::Number { .. })
    ) {
        return Err(Problem::field(
            "/unit",
            "invalid",
            "Only a number or a limit has a unit",
        ));
    }
    crate::v2::bounds::name(unit, "/unit")?;
    Ok(Some(unit.to_owned()))
}

/// The stored kind, policy and quantity of `kind`, its metric and quantity resolved.
async fn domain_type(
    state: &AppState,
    tenant_id: TenantId,
    kind: &FeatureType,
) -> Result<(DomainKind, Option<FeaturePolicy>, Option<ManagedQuantityId>), Problem> {
    Ok(match kind {
        FeatureType::Boolean(_) => (DomainKind::Boolean, None, None),
        FeatureType::Value(value) => {
            let select = value.value_type == FeatureValueType::Select;
            if value.higher_is_better.is_some() && value.value_type != FeatureValueType::Number {
                return Err(Problem::field(
                    "/type/higher_is_better",
                    "invalid",
                    "Only a number has a direction",
                ));
            }
            if !select && (value.options.is_some() || value.ordered.is_some()) {
                let path = if value.options.is_some() {
                    "/type/options"
                } else {
                    "/type/ordered"
                };
                return Err(Problem::field(path, "invalid", "Only a select has options"));
            }
            let value_type = match value.value_type {
                FeatureValueType::Number => ValueType::Number {
                    higher_is_better: value.higher_is_better.unwrap_or(true),
                },
                FeatureValueType::Text => ValueType::Text,
                FeatureValueType::Json => ValueType::Json,
                FeatureValueType::Select => {
                    let options = value.options.clone().unwrap_or_default();
                    check_options(&options)?;
                    ValueType::Select {
                        options,
                        ordered: value.ordered.unwrap_or(false),
                    }
                }
            };
            (DomainKind::Value(value_type), None, None)
        }
        FeatureType::Limit(limit) => {
            let counted_by = match (limit.counted_by, limit.metric.as_deref()) {
                (LimitCountedBy::Events, Some(raw)) => {
                    let metric = ids::metric(state, tenant_id, raw)
                        .await
                        .map_err(not_found_at("/type/metric", "metric", raw))?;
                    domain::LimitCountedBy::Events(metric.id)
                }
                (LimitCountedBy::Events, None) => {
                    return Err(Problem::field(
                        "/type/metric",
                        "required",
                        "A limit counted from events names its metric",
                    ));
                }
                (LimitCountedBy::App, Some(_)) => {
                    return Err(Problem::field(
                        "/type/metric",
                        "invalid",
                        "A limit counted by your app has no metric",
                    ));
                }
                (LimitCountedBy::App, None) => domain::LimitCountedBy::App,
            };
            let kind = DomainKind::Limit(counted_by);
            let policy = match &limit.policy {
                Some(policy) => Some(checked_policy(policy, &kind, "/type/policy")?),
                None => None,
            };
            let quantity = match limit.per_unit_of.as_deref() {
                Some(raw) => Some(quantity_of(state, tenant_id, raw).await?),
                None => None,
            };
            (kind, policy, quantity)
        }
    })
}

fn check_options(options: &[String]) -> Result<(), Problem> {
    if options.is_empty() {
        return Err(Problem::field(
            "/type/options",
            "required",
            "A select takes one option at least",
        ));
    }
    for (index, option) in options.iter().enumerate() {
        if option.trim().is_empty() || option.trim() != option {
            return Err(Problem::field(
                format!("/type/options/{index}"),
                "invalid",
                "An option is not empty and has no surrounding spaces",
            ));
        }
        if options[..index].contains(option) {
            return Err(Problem::field(
                format!("/type/options/{index}"),
                "duplicate",
                format!("'{option}' is listed twice"),
            ));
        }
    }
    Ok(())
}

/// What is read with features: their products', metrics', quantities' and resource types'
/// codes, each read once.
struct FeatureCodes {
    codes: Codes,
    metrics: HashMap<BillableMetricId, String>,
    quantities: HashMap<FeatureId, String>,
}

impl FeatureCodes {
    async fn load(
        state: &AppState,
        tenant_id: TenantId,
        features: &[domain::Feature],
    ) -> Result<Self, Problem> {
        let mut codes = Codes::of_features(state, tenant_id, features, std::iter::empty()).await?;
        codes
            .features
            .extend(features.iter().map(|f| (f.id, FeatureKey::from(f))));
        let mut metric_ids: Vec<BillableMetricId> =
            features.iter().filter_map(|f| f.kind.metric_id()).collect();
        metric_ids.sort_by_key(|id| id.as_uuid());
        metric_ids.dedup();
        let metrics = if metric_ids.is_empty() {
            HashMap::new()
        } else {
            state
                .store
                .find_billable_metrics_by_ids(&metric_ids, tenant_id)
                .await?
                .into_iter()
                .map(|metric| (metric.id, metric.catalog_code))
                .collect()
        };
        let quantities = if features
            .iter()
            .any(|f| matches!(f.kind, DomainKind::Limit(_)))
        {
            state
                .store
                .list_managed_quantities(tenant_id)
                .await?
                .into_iter()
                .flat_map(|quantity| {
                    let code = quantity.code;
                    quantity
                        .per_unit_features
                        .into_iter()
                        .map(move |feature| (feature, code.clone()))
                })
                .collect()
        } else {
            HashMap::new()
        };
        Ok(FeatureCodes {
            codes,
            metrics,
            quantities,
        })
    }

    fn kind(&self, feature: &domain::Feature) -> FeatureType {
        match &feature.kind {
            DomainKind::Boolean => FeatureType::Boolean(BooleanFeature {}),
            DomainKind::Value(value_type) => FeatureType::Value(match value_type {
                ValueType::Number { higher_is_better } => ValueFeature {
                    value_type: FeatureValueType::Number,
                    higher_is_better: Some(*higher_is_better),
                    options: None,
                    ordered: None,
                },
                ValueType::Text => ValueFeature {
                    value_type: FeatureValueType::Text,
                    higher_is_better: None,
                    options: None,
                    ordered: None,
                },
                ValueType::Json => ValueFeature {
                    value_type: FeatureValueType::Json,
                    higher_is_better: None,
                    options: None,
                    ordered: None,
                },
                ValueType::Select { options, ordered } => ValueFeature {
                    value_type: FeatureValueType::Select,
                    higher_is_better: None,
                    options: Some(options.clone()),
                    ordered: Some(*ordered),
                },
            }),
            DomainKind::Limit(counted_by) => FeatureType::Limit(LimitFeature {
                counted_by: match counted_by {
                    domain::LimitCountedBy::Events(_) => LimitCountedBy::Events,
                    domain::LimitCountedBy::App => LimitCountedBy::App,
                },
                metric: feature.kind.metric_id().map(|id| {
                    self.metrics
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| id.to_string())
                }),
                per_unit_of: self.quantities.get(&feature.id).cloned(),
                policy: Some(Policy::from(&feature.policy_or_default())),
            }),
        }
    }

    fn feature(&self, feature: domain::Feature) -> Feature {
        let kind = self.kind(&feature);
        let product = feature
            .product
            .as_ref()
            .map(|product| self.codes.refs.product(product.id));
        Feature {
            id: feature.id,
            code: feature.code,
            name: feature.name,
            description: feature.description,
            product,
            status: feature.status.into(),
            kind,
            unit: feature.unit,
            applies_per: feature.level_type_id.map(|id| self.codes.type_ref(id)),
            defaults: canonical(&self.codes, feature.defaults),
            created_at: feature.created_at.into(),
            updated_at: feature.updated_at.into(),
        }
    }
}

async fn to_api(
    state: &AppState,
    tenant_id: TenantId,
    features: Vec<domain::Feature>,
) -> Result<Vec<Feature>, Problem> {
    let codes = FeatureCodes::load(state, tenant_id, &features).await?;
    Ok(features
        .into_iter()
        .map(|feature| codes.feature(feature))
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

/// The feature written, as it reads; by its id, code, name and status when it is not read back
/// (a write committed never answers an error).
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

/// The feature of an id, or of a code: its live feature, else the one archived feature that
/// had it (several: named by id).
async fn found(
    state: &AppState,
    tenant_id: TenantId,
    raw: &str,
) -> Result<domain::Feature, Problem> {
    let raw = raw.trim();
    match ids::feature(state, tenant_id, raw).await {
        Ok(feature) => Ok(feature),
        Err(problem) if problem.status == 404 => {
            let AliasOr::Alias(code) = ids::id_or_code::<FeatureId>(raw) else {
                return Err(Problem::not_found(format!("feature {raw}")));
            };
            match state
                .store
                .find_archived_feature_by_code(&code, tenant_id)
                .await
            {
                Ok(id) => Ok(state.store.get_feature(id, tenant_id).await?),
                Err(error) => match error.current_context() {
                    StoreError::ValueNotFound(_) => {
                        Err(Problem::not_found(format!("feature {raw}")))
                    }
                    _ => Err(Problem::from(error)),
                },
            }
        }
        Err(problem) => Err(problem),
    }
}

fn archived_conflict(feature: &domain::Feature) -> Problem {
    Problem::conflict(
        ProblemCode::Conflict,
        format!(
            "The feature '{}' is archived: unarchive it first",
            feature.code
        ),
    )
}

/// The list query: `status` repeats, which the form extractor does not read.
fn list_query(raw: Option<&str>) -> Result<FeatureListQuery, Problem> {
    let mut query = FeatureListQuery::default();
    for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        let value = value.into_owned();
        match key.as_ref() {
            "product" => query.product = Some(value),
            "applies_per" => query.applies_per = Some(value),
            "after" => query.after = Some(value),
            "limit" => {
                query.limit =
                    Some(value.trim().parse::<u32>().map_err(|_| {
                        Problem::field("limit", "invalid", "limit is a whole number")
                    })?)
            }
            "status" => {
                let status = serde_json::from_value::<FeatureStatus>(serde_json::Value::String(
                    value.trim().to_owned(),
                ))
                .map_err(|_| {
                    Problem::field(
                        "status",
                        "invalid",
                        "status is `active`, `disabled` or `archived`",
                    )
                })?;
                query.status.get_or_insert_with(Vec::new).push(status);
            }
            _ => {}
        }
    }
    Ok(query)
}

// ── Handlers ────────────────────────────────────────────────────────

/// List features
///
/// Paged by keyset over the id.
#[utoipa::path(
    get,
    tag = "Features",
    path = "/api/v2/features",
    params(FeatureListQuery),
    responses(
        (status = 200, description = "A page of the tenant's features, archived ones left out unless asked", body = FeatureList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A parameter is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_features(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> Result<Json<FeatureList>, Problem> {
    let query = list_query(raw.as_deref())?;
    let product_id = match query.product.as_deref() {
        Some(product) => Some(
            ids::product_id(&state, auth.tenant_id, product)
                .await
                .map_err(not_found_at("product", "product", product))?,
        ),
        None => None,
    };
    let level = match query.applies_per.as_deref().map(str::trim) {
        None => None,
        Some("customer") => Some(None),
        Some(raw) => Some(Some(
            ids::resource_type(&state, auth.tenant_id, raw)
                .await
                .map_err(not_found_at("applies_per", "resource type", raw))?
                .id,
        )),
    };
    let page = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page.limit()?;
    let after = page.cursor()?.map(|Cursor(after)| FeatureId::from(after));
    let statuses = match query.status {
        Some(statuses) => statuses.into_iter().map(Into::into).collect(),
        None => vec![FeatureStatusEnum::Active, FeatureStatusEnum::Disabled],
    };
    let features = state
        .store
        .list_features_after(
            auth.tenant_id,
            FeaturePageFilter {
                statuses: Some(statuses),
                product_id,
                level,
                order: None,
            },
            after,
            limit + 1,
        )
        .await
        .map_err(|error| Problem::at("after", error))?;
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
/// By id, or by code: the live feature holding it, else the archived one that had it.
#[utoipa::path(
    get,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "The feature", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "Several archived features had the code: name one by id", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<Feature>, Problem> {
    let feature = found(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(one(&state, auth.tenant_id, feature).await?))
}

/// Create a feature
///
/// The feature, the managed quantity its limit counts per unit of and its default rows, in one
/// transaction.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features",
    request_body(
        content = CreateFeature,
        examples(
            ("events" = (summary = "A limit counted from a metric's events", value = json!({
                "code": "api_calls",
                "name": "API calls",
                "type": {
                    "type": "limit",
                    "counted_by": "events",
                    "metric": "api_calls",
                    "policy": {
                        "resets": {"type": "billing_cycle"},
                        "at_limit": "deny",
                        "grace_pct": 10,
                        "thresholds": [80]
                    }
                },
                "unit": "calls",
                "defaults": [{"feature": "api_calls", "value": {"type": "limit", "limit": "1000"}}]
            }))),
            ("app" = (summary = "A limit per site counted by your app", value = json!({
                "code": "projects",
                "name": "Projects",
                "type": {"type": "limit", "counted_by": "app"},
                "unit": "projects",
                "applies_per": "site"
            }))),
        )
    ),
    responses(
        (status = 201, description = "Created", body = Feature, examples(
            ("events" = (summary = "A limit counted from a metric's events", value = json!(examples::events()))),
            ("app" = (summary = "A limit per site counted by your app", value = json!(examples::app()))),
        )),
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
    let tenant_id = auth.tenant_id;
    let code = body.code.trim().to_owned();
    check_code(&code)?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "A feature has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    let product_id = match body.product.as_deref() {
        Some(product) => Some(product_of(&state, tenant_id, product).await?),
        None => None,
    };
    let (kind, policy, per_unit_of) = domain_type(&state, tenant_id, &body.kind).await?;
    let unit = unit_of(body.unit.as_deref(), &kind)?;
    let level_type_id = match body.applies_per.as_deref() {
        Some(raw) => Some(level_of(&state, tenant_id, raw, "/applies_per").await?),
        None => None,
    };
    // The defaults are checked against the feature as it is being created, under its own id.
    let now = common_clock::now();
    let id = FeatureId::new();
    let being_created = domain::Feature {
        id,
        tenant_id,
        product: product_id.map(|id| FeatureProductRef {
            id,
            name: String::new(),
        }),
        name: name.clone(),
        code: code.clone(),
        description: body.description.clone(),
        kind: kind.clone(),
        unit: unit.clone(),
        level_type_id,
        policy: policy.clone(),
        status: FeatureStatusEnum::Active,
        created_at: now,
        updated_at: now,
        defaults: Vec::new(),
    };
    let defaults = own_defaults(
        &state,
        tenant_id,
        &being_created,
        &body.defaults,
        "/defaults",
    )
    .await?;
    let created = state
        .store
        .create_feature_with_defaults(
            &auth.as_actor(),
            id,
            FeatureNew {
                tenant_id,
                product_id,
                name,
                code: code.clone(),
                description: body.description,
                kind,
                unit,
                level_type_id,
                policy,
                default: None,
            },
            per_unit_of,
            defaults,
        )
        .await
        .map_err(|error| {
            let problem = if body.defaults.is_empty() {
                Problem::from(error)
            } else {
                Problem::at("/defaults", error)
            };
            problem.code_taken(&code)
        })?;
    Ok(written(&state, tenant_id, StatusCode::CREATED, created).await)
}

/// The store's patch of `kind`'s fields that change, each checked against `current`; a fixed
/// field sent must read as it is.
async fn type_patch(
    state: &AppState,
    tenant_id: TenantId,
    current: &domain::Feature,
    patch: &UpdateFeatureType,
) -> Result<(FeatureUpdate, Option<Option<ManagedQuantityId>>), Problem> {
    let mut update = FeatureUpdate::default();
    if patch
        .kind
        .is_some_and(|kind| kind != FeatureKind::from(&current.kind))
    {
        return Err(fixed("/type/type", "type"));
    }
    let value_type = match &current.kind {
        DomainKind::Value(ValueType::Number { .. }) => Some(FeatureValueType::Number),
        DomainKind::Value(ValueType::Text) => Some(FeatureValueType::Text),
        DomainKind::Value(ValueType::Select { .. }) => Some(FeatureValueType::Select),
        DomainKind::Value(ValueType::Json) => Some(FeatureValueType::Json),
        _ => None,
    };
    if patch.value_type.is_some() && patch.value_type != value_type {
        return Err(fixed("/type/value_type", "value type"));
    }
    let counted_by = match &current.kind {
        DomainKind::Limit(domain::LimitCountedBy::Events(_)) => Some(LimitCountedBy::Events),
        DomainKind::Limit(domain::LimitCountedBy::App) => Some(LimitCountedBy::App),
        _ => None,
    };
    if patch.counted_by.is_some() && patch.counted_by != counted_by {
        return Err(fixed("/type/counted_by", "counting"));
    }
    if let Some(raw) = patch.metric.as_deref() {
        let metric = ids::metric(state, tenant_id, raw)
            .await
            .map_err(not_found_at("/type/metric", "metric", raw))?;
        if current.kind.metric_id() != Some(metric.id) {
            return Err(fixed("/type/metric", "metric"));
        }
    }
    if let Some(higher_is_better) = patch.higher_is_better {
        if value_type != Some(FeatureValueType::Number) {
            return Err(Problem::field(
                "/type/higher_is_better",
                "invalid",
                "Only a number has a direction",
            ));
        }
        update.number_higher_is_better = Some(higher_is_better);
    }
    if patch.options.is_some() || patch.ordered.is_some() {
        if value_type != Some(FeatureValueType::Select) {
            let path = if patch.options.is_some() {
                "/type/options"
            } else {
                "/type/ordered"
            };
            return Err(Problem::field(path, "invalid", "Only a select has options"));
        }
        if let Some(options) = &patch.options {
            check_options(options)?;
        }
        update.select_options = patch.options.clone();
        update.select_ordered = patch.ordered;
    }
    if let Some(policy) = &patch.policy {
        update.policy = Some(checked_policy(policy, &current.kind, "/type/policy")?);
    }
    let per_unit_of = match &patch.per_unit_of {
        None => None,
        Some(_) if counted_by.is_none() => {
            return Err(Problem::field(
                "/type/per_unit_of",
                "invalid",
                "Only a limit counts per unit of a quantity",
            ));
        }
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(quantity_of(state, tenant_id, raw).await?)),
    };
    Ok((update, per_unit_of))
}

/// `current` as `update` leaves it, which its new defaults are checked against.
fn as_updated(current: &domain::Feature, update: &FeatureUpdate) -> domain::Feature {
    let mut feature = current.clone();
    if let Some(level) = update.level_type_id {
        feature.level_type_id = level;
    }
    if let Some(unit) = &update.unit {
        feature.unit = unit.clone();
    }
    if let Some(policy) = &update.policy {
        feature.policy = Some(policy.clone());
    }
    match &mut feature.kind {
        DomainKind::Value(ValueType::Number { higher_is_better }) => {
            if let Some(new) = update.number_higher_is_better {
                *higher_is_better = new;
            }
        }
        DomainKind::Value(ValueType::Select { options, ordered }) => {
            if let Some(new) = &update.select_options {
                *options = new.clone();
            }
            if let Some(new) = update.select_ordered {
                *ordered = new;
            }
        }
        _ => {}
    }
    feature
}

/// Update a feature
///
/// A JSON merge patch of the feature as written, its fields, its quantity and its defaults in
/// one transaction. Its type, value type, counting and metric are fixed (`422 invalid` when sent
/// otherwise); an option a row still holds cannot be removed; `applies_per` changes only while
/// the feature has no rows.
#[utoipa::path(
    patch,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    request_body(content = UpdateFeature, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The feature is archived", body = Problem, content_type = "application/problem+json"),
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
    let tenant_id = auth.tenant_id;
    let current = found(&state, tenant_id, &id_or_code).await?;
    if current.status == FeatureStatusEnum::Archived {
        return Err(archived_conflict(&current));
    }
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
    // Every field is checked before the first write.
    let (mut update, per_unit_of) = match &body.kind {
        Some(patch) => type_patch(&state, tenant_id, &current, patch).await?,
        None => (FeatureUpdate::default(), None),
    };
    update.name = body.name.as_deref().map(|name| name.trim().to_owned());
    update.description = body.description.clone();
    update.product_id = match &body.product {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(product_of(&state, tenant_id, raw).await?)),
    };
    update.unit = match &body.unit {
        None => None,
        Some(unit) => Some(unit_of(unit.as_deref(), &current.kind)?),
    };
    update.level_type_id = match &body.applies_per {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(
            level_of(&state, tenant_id, raw, "/applies_per").await?,
        )),
    };
    let defaults = match &body.defaults {
        None => None,
        Some(inputs) => Some(
            own_defaults(
                &state,
                tenant_id,
                &as_updated(&current, &update),
                inputs,
                "/defaults",
            )
            .await?,
        ),
    };
    // A store refusal points at the one part that could refuse it, when only one is written.
    let mut parts: Vec<&str> = Vec::new();
    if update.select_options.is_some() || update.select_ordered.is_some() {
        parts.push("/type/options");
    }
    if update.level_type_id.is_some() {
        parts.push("/applies_per");
    }
    if update.product_id.is_some() {
        parts.push("/product");
    }
    if per_unit_of.is_some() {
        parts.push("/type/per_unit_of");
    }
    if defaults.is_some() {
        parts.push("/defaults");
    }
    let pointer = match parts.as_slice() {
        [one] => Some(*one),
        _ => None,
    };
    let changes_fields = update.name.is_some()
        || update.description.is_some()
        || update.product_id.is_some()
        || update.unit.is_some()
        || update.number_higher_is_better.is_some()
        || update.select_options.is_some()
        || update.select_ordered.is_some()
        || update.level_type_id.is_some()
        || update.policy.is_some();
    let updated = state
        .store
        .update_feature_with_defaults(
            &auth.as_actor(),
            current.id,
            tenant_id,
            changes_fields.then_some(update),
            per_unit_of,
            defaults,
        )
        .await
        .map_err(|error| match pointer {
            Some(pointer) => Problem::at(pointer, error),
            None => Problem::from(error),
        })?;
    Ok(written(&state, tenant_id, StatusCode::OK, updated).await)
}

/// Archive a feature
///
/// It entitles no one, its code is free for a new feature, and it and its rows are hidden until
/// it is unarchived. Archiving an archived feature changes nothing.
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
    let feature = found(&state, auth.tenant_id, &id_or_code).await?;
    if feature.status == FeatureStatusEnum::Archived {
        return Ok(Json(one(&state, auth.tenant_id, feature).await?).into_response());
    }
    state
        .store
        .set_feature_status(
            auth.as_actor(),
            feature.id,
            auth.tenant_id,
            FeatureStatusEnum::Archived,
        )
        .await?;
    Ok(reread(&state, auth.tenant_id, feature.id, FeatureStatus::Archived).await)
}

/// The code an archived feature had is held by a live feature.
fn code_taken(code: &str) -> Problem {
    let message = format!("A live feature holds the code '{code}': free it first");
    let mut taken = Problem::conflict(ProblemCode::CodeTaken, message.clone());
    taken.errors.push(FieldError {
        path: "/code".to_owned(),
        code: "duplicate".to_owned(),
        message,
    });
    taken
}

/// Unarchive a feature
///
/// Active again with its rows. By id, or by the code it had; `409 code_taken` while a live
/// feature holds that code. Unarchiving a feature not archived changes nothing.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "Unarchived", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "A live feature holds its code, or several archived features had it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    let feature = match ids::id_or_code::<FeatureId>(&id_or_code) {
        AliasOr::Id(id) => state.store.get_feature(id, tenant_id).await?,
        AliasOr::Alias(code) => {
            // A code a live feature holds names no archived feature to restore.
            if ids::feature(&state, tenant_id, &code).await.is_ok() {
                return Err(code_taken(&code));
            }
            found(&state, tenant_id, &code).await?
        }
    };
    if feature.status != FeatureStatusEnum::Archived {
        return Ok(Json(one(&state, tenant_id, feature).await?).into_response());
    }
    state
        .store
        .set_feature_status(
            auth.as_actor(),
            feature.id,
            tenant_id,
            FeatureStatusEnum::Active,
        )
        .await
        .map_err(|error| match error.current_context() {
            StoreError::FailedPrecondition(_) => code_taken(&feature.code),
            _ => Problem::from(error),
        })?;
    Ok(reread(&state, tenant_id, feature.id, FeatureStatus::Active).await)
}

/// Sets a live feature's status between active and disabled.
async fn switch(
    state: &AppState,
    auth: &AuthorizedAsTenant,
    id_or_code: &str,
    status: FeatureStatus,
) -> Result<Response, Problem> {
    let feature = found(state, auth.tenant_id, id_or_code).await?;
    if feature.status == FeatureStatusEnum::Archived {
        return Err(archived_conflict(&feature));
    }
    if FeatureStatus::from(feature.status.clone()) == status {
        return Ok(Json(one(state, auth.tenant_id, feature).await?).into_response());
    }
    state
        .store
        .set_feature_status(auth.as_actor(), feature.id, auth.tenant_id, status.into())
        .await?;
    Ok(reread(state, auth.tenant_id, feature.id, status).await)
}

/// Disable a feature
///
/// It entitles no one; its rows and code stay. Disabling a disabled feature changes nothing.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}/disable",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "Disabled", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The feature is archived", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn disable_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    switch(&state, &auth, &id_or_code, FeatureStatus::Disabled).await
}

/// Enable a feature
///
/// A disabled feature entitles again. Enabling an active feature changes nothing.
#[utoipa::path(
    post,
    tag = "Features",
    path = "/api/v2/features/{id_or_code}/enable",
    params(("id_or_code" = String, Path, description = "Feature id or code")),
    responses(
        (status = 200, description = "Enabled", body = Feature),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The feature is archived", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn enable_feature(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    switch(&state, &auth, &id_or_code, FeatureStatus::Active).await
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_features, create_feature))
        .routes(routes!(get_feature, update_feature))
        .routes(routes!(archive_feature))
        .routes(routes!(unarchive_feature))
        .routes(routes!(disable_feature))
        .routes(routes!(enable_feature))
}

/// The features the create operation's documentation shows, as they read back.
mod examples {
    use serde_json::{Value, json};

    const CREATED_AT: &str = "2026-11-26T10:15:00.000Z";

    pub fn events() -> Value {
        let feature = "feat_2xK9mQ4rT7vW1yZ3bN5cP8";
        json!({
            "id": feature,
            "code": "api_calls",
            "name": "API calls",
            "status": "active",
            "type": {
                "type": "limit",
                "counted_by": "events",
                "metric": "api_calls",
                "policy": {
                    "resets": {"type": "billing_cycle"},
                    "at_limit": "deny",
                    "grace_pct": 10,
                    "thresholds": [80]
                }
            },
            "unit": "calls",
            "defaults": [{
                "id": "ent_5gH7jK9lZ2xC4vB6nM8qW1",
                "feature": "api_calls",
                "value": {"type": "limit", "limit": "1000"},
                "effect": "set",
                "level": "customer",
                "owner": {"type": "feature", "id": feature},
                "created_by": {"type": "api_token", "id": "tkn_2aS4dF6gH8jK1lZ3xC5vB7"},
                "created_at": CREATED_AT,
                "updated_at": CREATED_AT
            }],
            "created_at": CREATED_AT,
            "updated_at": CREATED_AT
        })
    }

    pub fn app() -> Value {
        json!({
            "id": "feat_6hJ3kL8nP2qR5sT9vX1zB4",
            "code": "projects",
            "name": "Projects",
            "status": "active",
            "type": {
                "type": "limit",
                "counted_by": "app",
                "policy": {
                    "resets": {"type": "never"},
                    "at_limit": "deny_new",
                    "grace_pct": 0,
                    "thresholds": []
                }
            },
            "unit": "projects",
            "applies_per": {"id": "rty_5tY7uI9oP1aS3dF5gH7jK9", "code": "site"},
            "defaults": [],
            "created_at": CREATED_AT,
            "updated_at": CREATED_AT
        })
    }
}
