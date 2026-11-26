//! `/api/v2/metrics`: what usage is measured by. A metric's `code` is its own, unique per tenant
//! (archived metrics included) and immutable, the one every v2 path and reference names it by;
//! its `event_code` is the one the usage events it reads carry, which several metrics may share
//! (each aggregating the events its own way). A metric's dimensions are read by the rate cards
//! and dimension rates pricing it, not declared on it (v1's segmentation matrix is left as
//! stored).
use crate::AppState;
use crate::v2::extract::{Body, Path, Query};
use crate::v2::idempotency::committed_write;
use crate::v2::ids;
use crate::v2::pagination::{Cursor, PageQuery, keyset};
use crate::v2::problem::Problem;
use crate::v2::refs::{ProductFamilyRef, ProductRef, ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use common_domain::ids::{BaseId, BillableMetricId, ResourceTypeId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use http::StatusCode;
use meteroid_app::errors::RestApiError;
use meteroid_app::features::ensure_usage_breakdown_allowed;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::billable_metrics::{
    MetricFilter as DomainFilter, MetricFilterOp, MetricFilters,
};
use meteroid_store::domain::enums::{BillingMetricAggregateEnum, UnitConversionRoundingEnum};
use meteroid_store::domain::{
    AttributionPreview as DomainPreview, BillableMetric, BillableMetricNew, BillableMetricUpdate,
    MetricAttribution, MetricUnitLabel,
};
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    Count,
    Latest,
    Max,
    Min,
    Mean,
    Sum,
    CountDistinct,
}

impl From<BillingMetricAggregateEnum> for Aggregation {
    fn from(aggregation: BillingMetricAggregateEnum) -> Self {
        match aggregation {
            BillingMetricAggregateEnum::Count => Aggregation::Count,
            BillingMetricAggregateEnum::Latest => Aggregation::Latest,
            BillingMetricAggregateEnum::Max => Aggregation::Max,
            BillingMetricAggregateEnum::Min => Aggregation::Min,
            BillingMetricAggregateEnum::Mean => Aggregation::Mean,
            BillingMetricAggregateEnum::Sum => Aggregation::Sum,
            BillingMetricAggregateEnum::CountDistinct => Aggregation::CountDistinct,
        }
    }
}

impl From<Aggregation> for BillingMetricAggregateEnum {
    fn from(aggregation: Aggregation) -> Self {
        match aggregation {
            Aggregation::Count => BillingMetricAggregateEnum::Count,
            Aggregation::Latest => BillingMetricAggregateEnum::Latest,
            Aggregation::Max => BillingMetricAggregateEnum::Max,
            Aggregation::Min => BillingMetricAggregateEnum::Min,
            Aggregation::Mean => BillingMetricAggregateEnum::Mean,
            Aggregation::Sum => BillingMetricAggregateEnum::Sum,
            Aggregation::CountDistinct => BillingMetricAggregateEnum::CountDistinct,
        }
    }
}

/// How a period's usage is rounded once divided by `factor`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    Up,
    Down,
    /// Halves away from zero.
    Nearest,
    /// To the nearest 0.5.
    NearestHalf,
    /// To the nearest 0.1.
    NearestDecile,
    None,
}

impl From<UnitConversionRoundingEnum> for Rounding {
    fn from(rounding: UnitConversionRoundingEnum) -> Self {
        match rounding {
            UnitConversionRoundingEnum::Up => Rounding::Up,
            UnitConversionRoundingEnum::Down => Rounding::Down,
            UnitConversionRoundingEnum::Nearest => Rounding::Nearest,
            UnitConversionRoundingEnum::NearestHalf => Rounding::NearestHalf,
            UnitConversionRoundingEnum::NearestDecile => Rounding::NearestDecile,
            UnitConversionRoundingEnum::None => Rounding::None,
        }
    }
}

impl From<Rounding> for UnitConversionRoundingEnum {
    fn from(rounding: Rounding) -> Self {
        match rounding {
            Rounding::Up => UnitConversionRoundingEnum::Up,
            Rounding::Down => UnitConversionRoundingEnum::Down,
            Rounding::Nearest => UnitConversionRoundingEnum::Nearest,
            Rounding::NearestHalf => UnitConversionRoundingEnum::NearestHalf,
            Rounding::NearestDecile => UnitConversionRoundingEnum::NearestDecile,
            Rounding::None => UnitConversionRoundingEnum::None,
        }
    }
}

/// Usage divided by `factor` (tokens priced per 1M: 1000000), then each period's total rounded.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnitConversion {
    pub factor: i32,
    pub rounding: Rounding,
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Equal,
    NotEqual,
    In,
    NotIn,
}

/// Only the events whose `property` matches count. Filters are ANDed; a negation (`not_equal`,
/// `not_in`) leaves out an event without the property.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub property: String,
    pub op: FilterOp,
    pub values: Vec<String>,
}

/// Whose subscriptions bill the metric's events. `customer`: the customer's own. `resource_type`:
/// the subscriptions of the resource whose key the event carries (`property`, else the type's
/// keys, canonical first); an event naming no subscribed resource goes to the customer's own.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttributedTo {
    Customer(CustomerAttribution),
    ResourceType(ResourceTypeAttribution),
}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CustomerAttribution {}

#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceTypeAttribution {
    /// The resource type, by code (id or code on input).
    pub resource_type: String,
    /// The event property holding the resource's key; none: the type's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct Metric {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: BillableMetricId,
    /// Unique per tenant, immutable: what paths and references name the metric by.
    pub code: String,
    /// The code the usage events it reads carry; several metrics may read one.
    pub event_code: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub aggregation: Aggregation,
    /// The event property aggregated, for all but a count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_singular: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_plural: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_conversion: Option<UnitConversion>,
    pub filters: Vec<Filter>,
    /// The event property whose values are billed as lines of their own, each priced alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    /// The event property each invoice line is broken down by, still priced on its total.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breakdown_by: Option<String>,
    pub family: ProductFamilyRef,
    /// The product it measures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub attributed_to: AttributedTo,
    pub archived: bool,
    pub created_at: ApiDateTime,
}

crate::v2_list!(MetricList, Metric);

#[derive(Deserialize, IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct MetricListQuery {
    /// Only the metrics of this family, by code or id.
    pub family: Option<String>,
    /// Archived metrics too.
    #[serde(default)]
    pub include_archived: bool,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct CreateMetric {
    /// Unique per tenant, immutable; the event code, made unique (`-2`, `-3`…), when left out.
    #[serde(default)]
    pub code: Option<String>,
    /// The code the usage events it reads carry, immutable; `code` when left out. Several metrics
    /// may read one. One of `code` and `event_code` is sent at least.
    #[serde(default)]
    pub event_code: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Immutable.
    pub aggregation: Aggregation,
    /// The event property aggregated; required but for a count.
    #[serde(default)]
    pub aggregation_key: Option<String>,
    /// What one unit is called (`token`); sent with `unit_plural`.
    #[serde(default)]
    pub unit_singular: Option<String>,
    #[serde(default)]
    pub unit_plural: Option<String>,
    #[serde(default)]
    pub unit_conversion: Option<UnitConversion>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Immutable.
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(default)]
    pub breakdown_by: Option<String>,
    /// The product family, by code or id; the default family when left out.
    #[serde(default)]
    pub family: Option<String>,
    /// The product it measures, by code or id.
    #[serde(default)]
    pub product: Option<String>,
    /// The customer's own subscriptions when left out.
    #[serde(default)]
    pub attributed_to: Option<AttributedTo>,
}

/// A merge patch. The code, event code, aggregation, group, family and product never change.
#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct UpdateMetric {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Sent with `unit_plural`; both `null` remove the unit.
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub unit_singular: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub unit_plural: Option<Option<String>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<UnitConversion>)]
    pub unit_conversion: Option<Option<UnitConversion>>,
    /// Replaces every filter; `[]` removes them.
    #[serde(default)]
    pub filters: Option<Vec<Filter>>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schema(value_type = Option<String>)]
    pub breakdown_by: Option<Option<String>>,
    /// Applies to the periods not yet finalized: their draft invoices are recomputed. Fixed
    /// once a subscription for a resource has invoiced the metric, refused while one is live.
    #[serde(default)]
    pub attributed_to: Option<AttributedTo>,
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct PreviewAttribution {
    pub attributed_to: AttributedTo,
}

/// Where the last 7 days of the metric's usage would go, summed over up to 20 customers with
/// subscriptions for a resource (`sampled` when there are more): what each side's subscriptions
/// bill, each reading the metric's aggregation over its own events.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct AttributionPreview {
    pub from: ApiDateTime,
    pub until: ApiDateTime,
    pub sampled_customers: u32,
    pub sampled: bool,
    /// The three quantities add up to the usage (a sum or count metric) and split it in shares.
    /// False for latest, max, min, mean and distinct count: each is what its subscriptions bill,
    /// and they don't add up to the usage.
    pub additive: bool,
    /// Billed by subscriptions for a resource.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub resource_subscriptions: Decimal,
    /// Billed by the customer's own subscriptions.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub customer_subscriptions: Decimal,
    /// Billed by no subscription.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String, format = "decimal")]
    pub not_billed: Decimal,
    /// The draft invoices the change recomputes.
    pub draft_invoices: u32,
}

impl From<DomainPreview> for AttributionPreview {
    fn from(preview: DomainPreview) -> Self {
        AttributionPreview {
            from: preview.from.and_utc().into(),
            until: preview.until.and_utc().into(),
            sampled_customers: preview.sampled_customers,
            sampled: preview.sampled,
            additive: preview.additive,
            resource_subscriptions: preview.resource_subscriptions.normalize(),
            customer_subscriptions: preview.customer_subscriptions.normalize(),
            not_billed: preview.not_billed.normalize(),
            draft_invoices: preview.draft_invoices,
        }
    }
}

/// What `attributed_to` asks, its resource type resolved; the store checks it as v1's.
async fn attribution_of(
    state: &AppState,
    tenant_id: TenantId,
    attributed_to: &AttributedTo,
    path: &str,
) -> Result<Option<MetricAttribution>, Problem> {
    match attributed_to {
        AttributedTo::Customer(_) => Ok(None),
        AttributedTo::ResourceType(attribution) => {
            let raw = attribution.resource_type.as_str();
            let resource_type =
                ids::resource_type(state, tenant_id, raw)
                    .await
                    .map_err(|problem| {
                        if problem.status == 404 {
                            Problem::field(
                                format!("{path}/resource_type"),
                                "not_found",
                                format!("No resource type '{}'", raw.trim()),
                            )
                        } else {
                            problem
                        }
                    })?;
            Ok(Some(MetricAttribution {
                type_id: resource_type.id,
                property: attribution.property.clone(),
            }))
        }
    }
}

/// The codes of the resource types `metrics` are attributed to.
async fn type_codes(
    state: &AppState,
    tenant_id: TenantId,
    metrics: &[BillableMetric],
) -> Result<HashMap<ResourceTypeId, String>, Problem> {
    if metrics.iter().all(|m| m.attributed_to_type_id.is_none()) {
        return Ok(HashMap::new());
    }
    Ok(state
        .store
        .list_resource_types(tenant_id, true)
        .await?
        .into_iter()
        .map(|resource_type| (resource_type.id, resource_type.code))
        .collect())
}

fn attributed_to(m: &BillableMetric, types: &HashMap<ResourceTypeId, String>) -> AttributedTo {
    match m.attributed_to_type_id {
        None => AttributedTo::Customer(CustomerAttribution {}),
        Some(id) => AttributedTo::ResourceType(ResourceTypeAttribution {
            resource_type: types.get(&id).cloned().unwrap_or_else(|| id.to_string()),
            property: m.attribution_property.clone(),
        }),
    }
}

fn to_api(m: BillableMetric, refs: &Refs, types: &HashMap<ResourceTypeId, String>) -> Metric {
    let attributed_to = attributed_to(&m, types);
    let unit_conversion = m.unit_conversion_factor.map(|factor| UnitConversion {
        factor,
        rounding: m
            .unit_conversion_rounding
            .map(Rounding::from)
            .unwrap_or(Rounding::None),
    });
    Metric {
        id: m.id,
        code: m.catalog_code,
        event_code: m.code,
        name: m.name,
        description: m.description,
        aggregation: m.aggregation_type.into(),
        aggregation_key: m.aggregation_key,
        unit_singular: m.unit_singular,
        unit_plural: m.unit_plural,
        unit_conversion,
        filters: m
            .filters
            .map(|MetricFilters(filters)| filters.into_iter().map(filter_to_api).collect())
            .unwrap_or_default(),
        group_by: m.usage_group_key,
        breakdown_by: m.usage_breakdown_key,
        family: refs.product_family(m.product_family_id),
        product: m.product_id.map(|id| refs.product(id)),
        attributed_to,
        archived: m.archived_at.is_some(),
        created_at: m.created_at.into(),
    }
}

fn filter_to_api(filter: DomainFilter) -> Filter {
    Filter {
        property: filter.property,
        op: match filter.op {
            MetricFilterOp::Equal => FilterOp::Equal,
            MetricFilterOp::NotEqual => FilterOp::NotEqual,
            MetricFilterOp::In => FilterOp::In,
            MetricFilterOp::NotIn => FilterOp::NotIn,
        },
        values: filter.values,
    }
}

/// The filters as stored, none when empty; one without a property or a value is refused.
fn filters_of(filters: Vec<Filter>) -> Result<Option<MetricFilters>, Problem> {
    for (index, filter) in filters.iter().enumerate() {
        if filter.property.trim().is_empty() {
            return Err(Problem::field(
                format!("/filters/{index}/property"),
                "required",
                "A filter names a property",
            ));
        }
        if filter.values.is_empty() {
            return Err(Problem::field(
                format!("/filters/{index}/values"),
                "required",
                "A filter lists a value at least",
            ));
        }
    }
    if filters.is_empty() {
        return Ok(None);
    }
    Ok(Some(MetricFilters(
        filters
            .into_iter()
            .map(|filter| DomainFilter {
                property: filter.property.trim().to_owned(),
                op: match filter.op {
                    FilterOp::Equal => MetricFilterOp::Equal,
                    FilterOp::NotEqual => MetricFilterOp::NotEqual,
                    FilterOp::In => MetricFilterOp::In,
                    FilterOp::NotIn => MetricFilterOp::NotIn,
                },
                values: filter.values,
            })
            .collect(),
    )))
}

fn check_conversion(conversion: &UnitConversion) -> Result<(), Problem> {
    if conversion.factor < 1 {
        return Err(Problem::field(
            "/unit_conversion/factor",
            "invalid",
            "A conversion factor is 1 or more",
        ));
    }
    Ok(())
}

/// The codes of the products `metrics` measure.
/// The families and products the metrics name, read in one round.
async fn refs(
    state: &AppState,
    tenant_id: TenantId,
    metrics: &[BillableMetric],
) -> Result<Refs, Problem> {
    let ids = ReferenceIds {
        product_families: metrics.iter().map(|m| m.product_family_id).collect(),
        products: metrics.iter().filter_map(|m| m.product_id).collect(),
        ..ReferenceIds::default()
    };
    Refs::load(&state.store, tenant_id, &ids).await
}

async fn one(
    state: &AppState,
    tenant_id: TenantId,
    metric: BillableMetric,
) -> Result<Metric, Problem> {
    let refs = refs(state, tenant_id, std::slice::from_ref(&metric)).await?;
    let types = type_codes(state, tenant_id, std::slice::from_ref(&metric)).await?;
    Ok(to_api(metric, &refs, &types))
}

/// A metric written, as it reads; its family and product by id when their codes are not read back
/// (the write is made all the same: a write committed never answers an error).
async fn written(
    state: &AppState,
    tenant_id: TenantId,
    status: StatusCode,
    metric: BillableMetric,
) -> Response {
    let known = metric.clone();
    committed_write(status, one(state, tenant_id, metric).await, || {
        to_api(known, &Refs::default(), &HashMap::new())
    })
}

/// A metric archived or unarchived, read back; by its id when it is not.
async fn reread(state: &AppState, tenant_id: TenantId, metric: &BillableMetric) -> Response {
    let read = match state
        .store
        .find_billable_metric_by_id(metric.id, tenant_id)
        .await
    {
        Ok(found) => one(state, tenant_id, found).await,
        Err(error) => Err(error.into()),
    };
    committed_write(
        StatusCode::OK,
        read,
        || serde_json::json!({"id": metric.id.to_string(), "code": metric.catalog_code}),
    )
}

/// Usage breakdowns refused by the organization's plan, as v1 refuses them.
fn breakdown_refused<E>(refusal: E) -> Problem
where
    RestApiError: From<E>,
{
    Problem::from(RestApiError::from(refusal))
}

/// List metrics
#[utoipa::path(
    get,
    tag = "Metrics",
    path = "/api/v2/metrics",
    params(MetricListQuery),
    responses(
        (status = 200, description = "A page of the tenant's metrics", body = MetricList),
        (status = 401, description = "Unauthorized", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_metrics(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Query(query): Query<MetricListQuery>,
) -> Result<Json<MetricList>, Problem> {
    let family = match query.family.as_deref() {
        Some(family) => Some(ids::product_family_id(&state, auth.tenant_id, family).await?),
        None => None,
    };
    let page_query = PageQuery {
        limit: query.limit,
        after: query.after,
    };
    let limit = page_query.limit()?;
    let rows = state
        .store
        .list_billable_metrics_after(
            auth.tenant_id,
            family,
            query.include_archived,
            page_query
                .cursor()?
                .map(|Cursor(after)| BillableMetricId::from(after)),
            limit + 1,
        )
        .await?;
    let paged = keyset(rows, limit, |metric| metric.id.as_uuid());
    let refs = refs(&state, auth.tenant_id, &paged.data).await?;
    let types = type_codes(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(paged.map(|m| to_api(m, &refs, &types)).into()))
}

/// Get a metric
///
/// By id or code (its own, not its event code).
#[utoipa::path(
    get,
    tag = "Metrics",
    path = "/api/v2/metrics/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Metric id or code")),
    responses(
        (status = 200, description = "The metric", body = Metric),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_metric(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Json<Metric>, Problem> {
    let metric = ids::metric(&state, auth.tenant_id, &id_or_code).await?;
    Ok(Json(one(&state, auth.tenant_id, metric).await?))
}

/// Create a metric
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v2/metrics",
    request_body = CreateMetric,
    responses(
        (status = 201, description = "Created", body = Metric),
        (status = 403, description = "Usage breakdowns are not in your plan", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The code is taken", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn create_metric(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Body(body): Body<CreateMetric>,
) -> Result<Response, Problem> {
    let tenant_id = auth.tenant_id;
    ids::check_code(body.code.as_deref(), BillableMetricId::PREFIX)?;
    let event_code = match (body.event_code, body.code.as_deref()) {
        (Some(event_code), _) => event_code,
        (None, Some(code)) => code.to_owned(),
        (None, None) => {
            return Err(Problem::field(
                "/code",
                "required",
                "A metric has a code, an event code, or both",
            ));
        }
    };
    if event_code.trim().is_empty() || event_code.trim() != event_code {
        return Err(Problem::field(
            "/event_code",
            "invalid",
            "An event code is not empty and has no surrounding spaces",
        ));
    }
    common_domain::identifiers::validate_code(&event_code)
        .map_err(|error| Problem::field("/event_code", "invalid", error.to_string()))?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(Problem::field("/name", "required", "A metric has a name"));
    }
    crate::v2::bounds::name(&name, "/name")?;
    crate::v2::bounds::description(body.description.as_deref(), "/description")?;
    let aggregation_key = body
        .aggregation_key
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    if !matches!(body.aggregation, Aggregation::Count) && aggregation_key.is_none() {
        return Err(Problem::field(
            "/aggregation_key",
            "required",
            "The event property aggregated is named, but for a count",
        ));
    }
    if let Some(conversion) = &body.unit_conversion {
        check_conversion(conversion)?;
    }
    let unit_label =
        MetricUnitLabel::parse(body.unit_singular.as_deref(), body.unit_plural.as_deref())
            .map_err(|message| Problem::field("/unit_singular", "invalid", message))?;
    let filters = filters_of(body.filters)?;
    ensure_usage_breakdown_allowed(
        state.feature_resolver.as_ref(),
        auth.organization_id,
        body.breakdown_by.as_deref(),
        None,
    )
    .await
    .map_err(breakdown_refused)?;
    let family_id = crate::v2::product_families::family_for_new_use(
        &state,
        tenant_id,
        body.family.as_deref(),
        "/family",
    )
    .await?;
    let product_id = match body.product.as_deref() {
        Some(product) => Some(ids::product_id(&state, tenant_id, product).await.map_err(
            |problem| {
                if problem.status == 404 {
                    Problem::field(
                        "/product",
                        "not_found",
                        format!("product {product} not found"),
                    )
                } else {
                    problem
                }
            },
        )?),
        None => None,
    };
    let attribution = match &body.attributed_to {
        Some(asked) => attribution_of(&state, tenant_id, asked, "/attributed_to").await?,
        None => None,
    };
    let taken = body.code.clone().unwrap_or_default();
    let (unit_conversion_factor, unit_conversion_rounding) = match body.unit_conversion {
        Some(conversion) => (Some(conversion.factor), Some(conversion.rounding.into())),
        None => (None, None),
    };
    let metric = state
        .store
        .insert_billable_metric(
            auth.as_actor(),
            BillableMetricNew {
                name,
                description: body.description,
                code: event_code,
                catalog_code: body.code,
                aggregation_type: body.aggregation.into(),
                aggregation_key,
                unit_conversion_factor,
                unit_conversion_rounding,
                segmentation_matrix: None,
                filters,
                usage_group_key: body
                    .group_by
                    .map(|key| key.trim().to_owned())
                    .filter(|key| !key.is_empty()),
                usage_breakdown_key: body.breakdown_by,
                unit_label,
                tenant_id,
                product_family_id: family_id,
                product_id,
                attributed_to: attribution.clone(),
            },
        )
        .await
        // A create racing this one to the code meets the unique index of live codes.
        .map_err(|error| {
            let path = if attribution.is_some() {
                "/attributed_to"
            } else {
                "/code"
            };
            Problem::at(path, error).code_taken(&taken)
        })?;
    Ok(written(&state, tenant_id, StatusCode::CREATED, metric).await)
}

/// Update a metric
///
/// A JSON merge patch. The code, event code, aggregation, group, family and product never change;
/// the attribution changes the periods not yet finalized.
#[utoipa::path(
    patch,
    tag = "Metrics",
    path = "/api/v2/metrics/{id_or_code}",
    params(("id_or_code" = String, Path, description = "Metric id or code")),
    request_body(content = UpdateMetric, content_type = "application/merge-patch+json"),
    responses(
        (status = 200, description = "Updated", body = Metric),
        (status = 403, description = "Usage breakdowns are not in your plan", body = Problem, content_type = "application/problem+json"),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "A field is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_metric(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<UpdateMetric>,
) -> Result<Response, Problem> {
    let stored = ids::metric(&state, auth.tenant_id, &id_or_code).await?;
    if body
        .name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty())
    {
        return Err(Problem::field("/name", "invalid", "A metric has a name"));
    }
    if let Some(name) = body.name.as_deref() {
        crate::v2::bounds::name(name.trim(), "/name")?;
    }
    crate::v2::bounds::description(
        body.description.as_ref().and_then(|text| text.as_deref()),
        "/description",
    )?;
    let unit_label = match (body.unit_singular, body.unit_plural) {
        (None, None) => None,
        (Some(singular), Some(plural)) => Some(
            MetricUnitLabel::parse(singular.as_deref(), plural.as_deref())
                .map_err(|message| Problem::field("/unit_singular", "invalid", message))?,
        ),
        _ => {
            return Err(Problem::field(
                "/unit_plural",
                "required",
                "unit_singular and unit_plural are sent together",
            ));
        }
    };
    let (unit_conversion_factor, unit_conversion_rounding) = match body.unit_conversion {
        None => (None, None),
        Some(None) => (Some(None), Some(None)),
        Some(Some(conversion)) => {
            check_conversion(&conversion)?;
            (
                Some(Some(conversion.factor)),
                Some(Some(conversion.rounding.into())),
            )
        }
    };
    let filters = match body.filters {
        None => None,
        Some(filters) => Some(filters_of(filters)?),
    };
    if let Some(Some(requested)) = &body.breakdown_by {
        ensure_usage_breakdown_allowed(
            state.feature_resolver.as_ref(),
            auth.organization_id,
            Some(requested),
            stored.usage_breakdown_key.as_deref(),
        )
        .await
        .map_err(breakdown_refused)?;
    }
    let attribution = match &body.attributed_to {
        Some(asked) => Some(attribution_of(&state, auth.tenant_id, asked, "/attributed_to").await?),
        None => None,
    };
    let pointer = if attribution.is_some() {
        "/attributed_to"
    } else {
        "/breakdown_by"
    };
    // As v1: a changed attribution marks the drafts it moves usage of.
    let metric = state
        .services
        .update_billable_metric(
            auth.as_actor(),
            stored.id,
            auth.tenant_id,
            BillableMetricUpdate {
                name: body.name.map(|name| name.trim().to_owned()),
                description: body.description,
                unit_conversion_factor,
                unit_conversion_rounding,
                segmentation_matrix: None,
                filters,
                unit_label,
                usage_breakdown_key: body.breakdown_by,
                attributed_to: attribution,
            },
        )
        .await
        .map_err(|error| Problem::at(pointer, error))?;
    Ok(written(&state, auth.tenant_id, StatusCode::OK, metric).await)
}

/// Archive a metric
///
/// Its code stays taken. Archiving an archived metric changes nothing.
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v2/metrics/{id_or_code}/archive",
    params(("id_or_code" = String, Path, description = "Metric id or code")),
    responses(
        (status = 200, description = "Archived", body = Metric),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 409, description = "The metric's state refuses it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn archive_metric(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let metric = ids::metric(&state, auth.tenant_id, &id_or_code).await?;
    if metric.archived_at.is_some() {
        return Ok(Json(one(&state, auth.tenant_id, metric).await?).into_response());
    }
    state
        .store
        .archive_billable_metric(auth.as_actor(), metric.id, auth.tenant_id)
        .await?;
    Ok(reread(&state, auth.tenant_id, &metric).await)
}

/// Unarchive a metric
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v2/metrics/{id_or_code}/unarchive",
    params(("id_or_code" = String, Path, description = "Metric id or code")),
    responses(
        (status = 200, description = "Unarchived", body = Metric),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn unarchive_metric(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
) -> Result<Response, Problem> {
    let metric = ids::metric(&state, auth.tenant_id, &id_or_code).await?;
    if metric.archived_at.is_none() {
        return Ok(Json(one(&state, auth.tenant_id, metric).await?).into_response());
    }
    state
        .store
        .unarchive_billable_metric(metric.id, auth.tenant_id)
        .await?;
    Ok(reread(&state, auth.tenant_id, &metric).await)
}

/// Preview a metric's attribution
///
/// Where the last 7 days of its usage would go under `attributed_to`, sampled over 20 customers
/// with subscriptions for a resource, and the draft invoices the change would recompute. Nothing
/// is written.
#[utoipa::path(
    post,
    tag = "Metrics",
    path = "/api/v2/metrics/{id_or_code}/attribution/preview",
    operation_id = "preview_metric_attribution",
    params(("id_or_code" = String, Path, description = "Metric id or code")),
    request_body = PreviewAttribution,
    responses(
        (status = 200, description = "Where its usage would go", body = AttributionPreview),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
        (status = 422, description = "The attribution is refused", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn preview_attribution(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_code): Path<String>,
    Body(body): Body<PreviewAttribution>,
) -> Result<Json<AttributionPreview>, Problem> {
    let metric = ids::metric(&state, auth.tenant_id, &id_or_code).await?;
    let attribution = attribution_of(
        &state,
        auth.tenant_id,
        &body.attributed_to,
        "/attributed_to",
    )
    .await?;
    let preview = state
        .services
        .preview_metric_attribution(auth.tenant_id, metric.id, attribution)
        .await
        .map_err(|error| Problem::at("/attributed_to", error))?;
    Ok(Json(preview.into()))
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_metrics, create_metric))
        .routes(routes!(get_metric, update_metric))
        .routes(routes!(archive_metric))
        .routes(routes!(unarchive_metric))
        .routes(routes!(preview_attribution))
}
