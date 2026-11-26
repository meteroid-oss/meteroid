//! What a customer may use (ADR-0013 §23): the snapshot of every feature at each level (the
//! customer, and the resource the request names), with its `ETag`, one feature of it, and the
//! usage of a limit the customer's app counts and reports.
use crate::AppState;
use crate::entitlements::mapping::bracketed_params;
use crate::v2::entitlements::model::{
    Codes, EntitlementValue, FeatureKind as ApiFeatureKind, LevelKind, LevelResource, Origin,
    Policy, UsageWindow, level_kind, level_resource,
};
use crate::v2::extract::{Body, Path};
use crate::v2::idempotency::IDEMPOTENCY_KEY_HEADER;
use crate::v2::ids;
use crate::v2::problem::Problem;
use crate::v2::refs::{FeatureRef, ProductRef, ResourceTypeRef, SubscriptionRef};
use axum::Extension;
use axum::Json;
use axum::extract::{RawQuery, State};
use axum::response::{IntoResponse, Response};
use common_domain::ids::{CustomerId, ResourceId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlement_checks::{
    FeatureScope as DomainScope, LevelValue, Snapshot, SnapshotFeature, SnapshotLevel,
    SnapshotRead, usage_warnings,
};
use meteroid_store::domain::entitlements::{Feature, FeatureKind, LimitCountedBy};
use meteroid_store::domain::resources::ResourceInput;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::services::entitlements::app_counts::{AppCountOp, AppCountWrite};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

use super::checks::CheckReason;
use super::resources::{ResourceName, query_resource, resource_input};

/// The resource and attributes a v2 query names, as the store reads them.
pub(crate) async fn query_input(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    query: Option<&str>,
) -> Result<ResourceInput, Problem> {
    let resource = query_resource(query)?;
    resource_input(
        state,
        tenant_id,
        customer_id,
        resource.as_ref(),
        bracketed_params(query, "attributes"),
        "resource",
    )
    .await
}

/// Which subscriptions include a feature: the customer's (its value holds for the whole
/// customer), or only its resources' (it is answered per resource: name one).
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncludedBy {
    Customer,
    Resources,
}

impl From<DomainScope> for IncludedBy {
    fn from(scope: DomainScope) -> Self {
        match scope {
            DomainScope::Customer => IncludedBy::Customer,
            DomainScope::Resource => IncludedBy::Resources,
        }
    }
}

/// A feature at one level: the customer as a whole, or the resource the request names.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct EntitlementLevel {
    pub level: LevelKind,
    /// With `level: resource_type`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<ResourceTypeRef>,
    /// With `level: resource_type`: the resource this level is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    /// The level the feature applies per (`applies_per`, else the customer): its value is the
    /// feature's, and none there means not entitled. The other levels can only cap it.
    pub primary: bool,
    /// Absent on the primary level: not entitled (`reason` tells when a resource is needed).
    /// Absent on another level: nothing caps the value there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    /// `resource_required` or `not_subscribed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CheckReason>,
    /// A limit's policy: the feature's, or the one a `set` row gives in its place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// A limit's usage: the count your app reported, or the metric's events for this level over
    /// `window`. Absent when it could not be read (warning `usage_unavailable`).
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<UsageWindow>,
    /// The subscription whose value applies here, when several include the feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_subscription: Option<SubscriptionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// A feature as the customer may use it.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CustomerEntitlement {
    pub feature: FeatureRef,
    /// The feature's name.
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ApiFeatureKind,
    /// What a number or a limit counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// The product the feature belongs to; none for a feature of the whole catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    /// The resource type the feature applies per; none: the customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_per: Option<ResourceTypeRef>,
    pub included_by: IncludedBy,
    /// The primary level's value, capped by the other levels; absent: not entitled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    /// The primary level first.
    pub levels: Vec<EntitlementLevel>,
}

/// One feature as the customer may use it, read alone.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct CustomerEntitlementRead {
    #[serde(flatten)]
    pub entitlement: CustomerEntitlement,
    /// `usage_unavailable`: a level's usage could not be read, its `used` is absent.
    pub warnings: Vec<String>,
}

/// The resource a snapshot is for.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct SnapshotResource {
    /// The type's code.
    #[serde(rename = "type")]
    pub resource_type: String,
    /// The key value named.
    pub key: String,
    /// A stored resource's id; absent for one named ad hoc.
    #[serde(
        serialize_with = "common_domain::ids::string_serde_opt::serialize",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<ResourceId>,
}

/// Every feature with its levels. `etag` is also the `ETag` header: sent back as
/// `If-None-Match`, it answers `304` while the values are the same (usage is not part of it).
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct EntitlementSnapshot {
    pub etag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<SnapshotResource>,
    pub entitlements: Vec<CustomerEntitlement>,
    /// Attributes the request asserted that the stored resource holds other values of: the
    /// stored ones were used.
    pub attributes_ignored: Vec<String>,
    /// `usage_unavailable`: a level's usage could not be read, its `used` is absent.
    pub warnings: Vec<String>,
}

fn level_of(feature: &Feature, level: SnapshotLevel, codes: &Codes) -> EntitlementLevel {
    let (kind, resource_type) = level_kind(level.level, codes);
    let (value, reason): (Option<EntitlementValue>, Option<CheckReason>) = match &level.value {
        LevelValue::Value(value) => (Some(value.into()), None),
        LevelValue::Empty => (None, None),
        LevelValue::ResourceRequired => (None, Some(CheckReason::ResourceRequired)),
        LevelValue::NotSubscribed => (None, Some(CheckReason::NotSubscribed)),
    };
    let limit = matches!(feature.kind, FeatureKind::Limit(_));
    let counted_from_events = matches!(feature.kind, FeatureKind::Limit(LimitCountedBy::Events(_)));
    EntitlementLevel {
        level: kind,
        resource_type,
        primary: level.level == feature.own_level(),
        resource: level_resource(&level.path),
        value,
        reason,
        policy: limit.then(|| (&level.policy).into()),
        used: if limit { level.used } else { None },
        window: level
            .window
            .as_ref()
            .filter(|_| counted_from_events)
            .map(Into::into),
        deciding_subscription: level.deciding_subscription.map(|id| codes.subscription(id)),
        origin: level.origin.as_ref().map(Into::into),
    }
}

pub(crate) fn customer_entitlement(feature: SnapshotFeature, codes: &Codes) -> CustomerEntitlement {
    let SnapshotFeature {
        feature,
        scope,
        value,
        levels,
    } = feature;
    let facts = codes.facts(&feature);
    CustomerEntitlement {
        feature: facts.feature,
        name: facts.name,
        kind: facts.kind,
        unit: facts.unit,
        product: facts.product,
        applies_per: facts.applies_per,
        included_by: scope.into(),
        value: value.as_ref().map(Into::into),
        levels: levels
            .into_iter()
            .map(|level| level_of(&feature, level, codes))
            .collect(),
    }
}

/// The subscriptions deciding a level of `features`.
pub(crate) fn deciding<'a>(
    features: impl IntoIterator<Item = &'a SnapshotFeature>,
) -> Vec<SubscriptionId> {
    features
        .into_iter()
        .flat_map(|feature| &feature.levels)
        .filter_map(|level| level.deciding_subscription)
        .collect()
}

async fn path_resource(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    input: &ResourceInput,
) -> Result<Option<SnapshotResource>, Problem> {
    if input.resources.is_empty() {
        return Ok(None);
    }
    Ok(state
        .store
        .resolve_resource_input(tenant_id, customer_id, input)
        .await?
        .map(|resolved| SnapshotResource {
            resource_type: resolved.type_code,
            key: resolved.key_value,
            id: resolved.reference.stored_id(),
        }))
}

fn etag_header(etag: &str) -> Result<HeaderValue, Problem> {
    HeaderValue::from_str(&format!("\"{etag}\"")).map_err(|_| Problem::internal())
}

async fn snapshot_of(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    input: &ResourceInput,
    snapshot: Snapshot,
) -> Result<EntitlementSnapshot, Problem> {
    let features = snapshot.features.iter().map(|f| &f.feature);
    let codes = Codes::of_features(
        state,
        tenant_id,
        features,
        deciding(snapshot.features.iter()),
    )
    .await?;
    Ok(EntitlementSnapshot {
        etag: snapshot.etag,
        resource: path_resource(state, tenant_id, customer_id, input).await?,
        entitlements: snapshot
            .features
            .into_iter()
            .map(|feature| customer_entitlement(feature, &codes))
            .collect(),
        attributes_ignored: snapshot.attributes_ignored,
        warnings: snapshot.warnings,
    })
}

/// The snapshot as answered, with its `ETag`, or `304` when `If-None-Match` names it. A
/// resource or attribute the store refuses is refused at `resource_field`.
pub(crate) async fn snapshot_response(
    state: &AppState,
    tenant_id: TenantId,
    customer_id: CustomerId,
    input: ResourceInput,
    headers: &HeaderMap,
    resource_field: &str,
) -> Result<Response, Problem> {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    let read = state
        .store
        .entitlement_snapshot(tenant_id, customer_id, &input, if_none_match)
        .await
        .map_err(|error| Problem::at(resource_field, error))?;
    Ok(match read {
        SnapshotRead::NotModified { etag } => (
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, etag_header(&etag)?)],
        )
            .into_response(),
        SnapshotRead::Modified(snapshot) => {
            let etag = etag_header(&snapshot.etag)?;
            let body = snapshot_of(state, tenant_id, customer_id, &input, snapshot).await?;
            ([(header::ETAG, etag)], Json(body)).into_response()
        }
    })
}

/// A plan granting SSO, and 5 projects per site with at most 50 across all sites.
fn snapshot_example() -> serde_json::Value {
    let site = serde_json::json!({"id": "rty_5tY7uI9oP1aS3dF5gH7jK9", "code": "site"});
    let subscription = serde_json::json!({"id": "sub_3zX5cV7bN9mQ1wE3rT5yU7"});
    let policy = serde_json::json!({
        "resets": {"type": "never"},
        "at_limit": "deny_new",
        "grace_pct": 0,
        "thresholds": []
    });
    serde_json::json!({
        "etag": "3f2a9c1e7b6d4e50",
        "resource": {"type": "site", "key": "s1", "id": "res_8qW2eR4tY6uI8oP1aS3dF5"},
        "entitlements": [
            {
                "feature": {"id": "feat_1aB3cD5eF7gH9iJ2kL4mN6", "code": "sso"},
                "name": "SSO",
                "type": "boolean",
                "included_by": "customer",
                "value": {"type": "on"},
                "levels": [{
                    "level": "customer",
                    "primary": true,
                    "value": {"type": "on"},
                    "deciding_subscription": subscription
                }]
            },
            {
                "feature": {"id": "feat_6hJ3kL8nP2qR5sT9vX1zB4", "code": "projects"},
                "name": "Projects",
                "type": "limit",
                "unit": "projects",
                "applies_per": site,
                "included_by": "customer",
                "value": {"type": "limit", "limit": "5"},
                "levels": [
                    {
                        "level": "resource_type",
                        "resource_type": site,
                        "resource": {"type": "site", "key": "s1"},
                        "primary": true,
                        "value": {"type": "limit", "limit": "5"},
                        "policy": policy,
                        "used": "3",
                        "deciding_subscription": subscription
                    },
                    {
                        "level": "customer",
                        "primary": false,
                        "value": {"type": "limit", "limit": "50"},
                        "policy": policy,
                        "used": "32",
                        "deciding_subscription": subscription
                    }
                ]
            }
        ],
        "attributes_ignored": [],
        "warnings": []
    })
}

/// List what a customer may use
///
/// Every feature, once, at each level: the customer as a whole, and the resource named by
/// `resource` (a stored resource's id) or by `resource[type]` and `resource[key]` (stored or
/// not). The `ETag` changes when a value changes, not with usage: send it back as
/// `If-None-Match` to get `304` while nothing changed. A level whose usage cannot be read comes
/// without `used`, and the answer carries the warning `usage_unavailable`.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements",
    operation_id = "get_customer_entitlements",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("resource" = Option<String>, Query, description = "A stored resource of the customer, by id"),
        ("resource[type]" = Option<String>, Query, description = "With `resource[key]`: a resource, stored or not, by its type's code"),
        ("resource[key]" = Option<String>, Query, description = "With `resource[type]`: the resource's key value"),
        ("attributes" = Option<BTreeMap<String, String>>, Query, style = DeepObject, explode, description = "Asserted attributes, as `attributes[<type>.<attribute>]=<value>`; a stored resource's own win"),
        ("If-None-Match" = Option<String>, Header, description = "The `ETag` of a previous answer"),
    ),
    responses(
        (status = 200, description = "The snapshot", body = EntitlementSnapshot, examples(
            ("site" = (summary = "For the site s1: `?resource[type]=site&resource[key]=s1`", value = json!(snapshot_example()))),
        )),
        (status = 304, description = "Unchanged since the `If-None-Match` etag"),
        (status = 404, description = "The customer is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A resource not the customer's or of no type, or an unknown attribute or value", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The snapshot could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_entitlements(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Result<Response, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let input = query_input(&state, auth.tenant_id, customer.id, query.as_deref()).await?;
    snapshot_response(
        &state,
        auth.tenant_id,
        customer.id,
        input,
        &headers,
        "resource",
    )
    .await
}

/// Get what a customer may use of a feature
///
/// One feature of the snapshot (code or id), with the same query and the same warnings; it
/// carries no `ETag`.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements/{feature}",
    operation_id = "get_customer_entitlement",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("feature" = String, Path, description = "Feature code or id"),
        ("resource" = Option<String>, Query, description = "A stored resource of the customer, by id"),
        ("resource[type]" = Option<String>, Query, description = "With `resource[key]`: a resource, stored or not, by its type's code"),
        ("resource[key]" = Option<String>, Query, description = "With `resource[type]`: the resource's key value"),
        ("attributes" = Option<BTreeMap<String, String>>, Query, style = DeepObject, explode, description = "Asserted attributes, as `attributes[<type>.<attribute>]=<value>`"),
    ),
    responses(
        (status = 200, description = "The feature as the customer may use it", body = CustomerEntitlementRead),
        (status = 404, description = "The customer or the feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A resource not the customer's or of no type, or an unknown attribute or value", body = Problem, content_type = "application/problem+json"),
        (status = 503, description = "The feature could not be answered in time; retry", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_entitlement(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, feature)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Result<Json<CustomerEntitlementRead>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let feature = ids::feature(&state, auth.tenant_id, &feature).await?;
    let input = query_input(&state, auth.tenant_id, customer.id, query.as_deref()).await?;
    let found = state
        .store
        .entitlement_snapshot_features(auth.tenant_id, customer.id, &input, &[feature.id])
        .await
        .map_err(|error| Problem::at("resource", error))?
        .into_iter()
        .find(|f| f.feature.id == feature.id)
        .ok_or_else(|| {
            Problem::not_found(format!("The feature '{}' is not active", feature.code))
        })?;
    let codes = Codes::of_features(
        &state,
        auth.tenant_id,
        std::iter::once(&found.feature),
        deciding(std::iter::once(&found)),
    )
    .await?;
    Ok(Json(CustomerEntitlementRead {
        warnings: usage_warnings(std::iter::once(&found)),
        entitlement: customer_entitlement(found, &codes),
    }))
}

// ── Usage counted by the app ────────────────────────────────────────

/// A count your app reports for a limit it counts: `set` the count, or `add` to it (negative to
/// remove), for the customer or for one resource.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ReportUsage {
    /// The count now, not negative.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub set: Option<Decimal>,
    /// A change of the count.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub add: Option<Decimal>,
    /// The resource counted, stored or not: `{"id": …}`, or `{"type": …, "key": …}`. Left out:
    /// the customer as a whole.
    #[serde(default)]
    pub resource: Option<ResourceName>,
    /// When the count was true; the time received by default.
    #[serde(default)]
    pub at: Option<ApiDateTime>,
}

/// The count after a report.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct AppCount {
    pub feature: FeatureRef,
    /// The resource counted; absent for the customer as a whole.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub count: Decimal,
    /// False for a report older than the last `set`: recorded, and changing nothing.
    pub applied: bool,
    /// The store had the key: its first answer, nothing applied again.
    pub replayed: bool,
}

/// The store keeps an operation's key with its count (`entitlement_app_count_op`).
const MAX_OPERATION_KEY_BYTES: usize = 255;

fn operation_key(headers: &HeaderMap) -> Result<String, Problem> {
    let key = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .map(|value| value.to_str().map(str::trim))
        .ok_or_else(|| {
            Problem::key_required(
                "A count is reported with an Idempotency-Key: a retry must never count twice",
            )
        })?
        .map_err(|_| {
            Problem::field(
                IDEMPOTENCY_KEY_HEADER,
                "invalid",
                "An Idempotency-Key is visible ASCII",
            )
        })?;
    if key.is_empty() {
        return Err(Problem::key_required(
            "A count is reported with an Idempotency-Key: a retry must never count twice",
        ));
    }
    if key.len() > MAX_OPERATION_KEY_BYTES {
        return Err(Problem::field(
            IDEMPOTENCY_KEY_HEADER,
            "invalid",
            format!("An Idempotency-Key is 1 to {MAX_OPERATION_KEY_BYTES} bytes"),
        ));
    }
    Ok(key.to_owned())
}

/// Report usage counted by your app
///
/// For a limit your app counts: `set` the count, or `add` to it, for the customer or one
/// resource. An `Idempotency-Key` is required and kept with the count for the tenant's retention
/// (7 days by default): a retry answers its first count, applying nothing. A report older than
/// the last `set` is recorded and changes nothing.
#[utoipa::path(
    post,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements/{feature}/usage",
    operation_id = "report_entitlement_usage",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("feature" = String, Path, description = "Feature code or id"),
    ),
    request_body(
        content = ReportUsage,
        examples(
            ("set" = (summary = "The site's count now", value = json!({"set": "3", "resource": {"type": "site", "key": "s1"}}))),
            ("add" = (summary = "One project created on the site", value = json!({"add": "1", "resource": {"type": "site", "key": "s1"}}))),
        )
    ),
    responses(
        (status = 200, description = "The count after the report", body = AppCount, example = json!({
            "feature": {"id": "feat_6hJ3kL8nP2qR5sT9vX1zB4", "code": "projects"},
            "resource": {"type": "site", "key": "s1"},
            "count": "4",
            "applied": true,
            "replayed": false
        })),
        (status = 404, description = "The customer or the feature is not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "`stale_set`: a set older than the operations kept", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "Not a limit counted by your app, none or both of `set` and `add`, a resource missing (`resource_required`) or invalid, or the key used with another report (`idempotency_key_reused`)", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn report_usage(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, feature)): Path<(String, String)>,
    headers: HeaderMap,
    Body(body): Body<ReportUsage>,
) -> Result<Json<AppCount>, Problem> {
    let idempotency_key = operation_key(&headers)?;
    let op = match (body.set, body.add) {
        (Some(set), None) if set.is_sign_negative() => {
            return Err(Problem::field("/set", "invalid", "A count is not negative"));
        }
        (Some(set), None) => AppCountOp::Set(set),
        (None, Some(add)) => AppCountOp::Add(add),
        _ => {
            return Err(Problem::field(
                "/set",
                "invalid",
                "Send exactly one of `set` and `add`",
            ));
        }
    };
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let input = resource_input(
        &state,
        auth.tenant_id,
        customer.id,
        body.resource.as_ref(),
        BTreeMap::new(),
        "/resource",
    )
    .await?;
    let feature = ids::feature(&state, auth.tenant_id, &feature).await?;
    if !feature.kind.counted_by_app() {
        return Err(Problem::field(
            "feature",
            "invalid",
            format!("'{}' is not a limit counted by your app", feature.code),
        ));
    }
    let result = state
        .store
        .report_app_count(
            auth.tenant_id,
            customer.id,
            feature.id,
            AppCountWrite {
                resources: input.resources,
                op,
                at: body.at.map(Into::into),
                idempotency_key,
            },
        )
        .await
        .map_err(|error| Problem::at("/resource", error))?;
    Ok(Json(AppCount {
        feature: FeatureRef {
            id: feature.id,
            code: feature.code,
        },
        resource: level_resource(&result.path),
        count: result.count.normalize(),
        applied: result.applied,
        replayed: result.replayed,
    }))
}
