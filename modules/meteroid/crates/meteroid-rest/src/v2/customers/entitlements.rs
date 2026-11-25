//! `GET /api/v2/customers/{id_or_alias}/entitlements[/{feature}]`: what the customer may use, each
//! feature's entitlement merged from its subscriptions, add-ons, overrides and the feature's
//! default, with the usage counted against a metered limit. Features are named by code, metrics
//! and products by code too. A subscription's entitlements read in the same shape
//! (`subscriptions::entitlements`).
use crate::AppState;
use crate::v2::entitlements::Reset;
use crate::v2::extract::{Path, Query};
use crate::v2::ids;
use crate::v2::pagination::{PageQuery, page};
use crate::v2::problem::Problem;
use crate::v2::refs::{FeatureRef, MetricRef, ProductRef, ReferenceIds, Refs};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{AliasOr, BaseId, FeatureId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_rest_common::datetime::ApiDateTime;
use meteroid_store::domain::entitlements::{
    ConfigValue, EffectiveEntitlement, EffectiveEntitlementValue, OverageBehavior,
};
use meteroid_store::repositories::EntitlementsInterface;
use rust_decimal::Decimal;
use serde::Serialize;
use std::str::FromStr;
use utoipa::ToSchema;

/// What reaching the limit does: refuse (past `grace_pct` percent more), or let usage go on.
#[derive(Serialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AtLimit {
    Block,
    Allow,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct EntitlementUsage {
    /// Counted in the current window; none when the usage could not be read.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub consumed: Option<Decimal>,
    /// Left under the limit.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub remaining: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_start: Option<ApiDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<ApiDateTime>,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct MeteredEntitlement {
    /// Switched on and not refused by a reached blocking limit.
    pub enabled: bool,
    /// The metric counting the usage.
    pub metric: MetricRef,
    /// A decimal string; none is unlimited.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub limit: Option<Decimal>,
    pub reset: Reset,
    pub at_limit: AtLimit,
    /// With `block`: how far past the limit usage may go before it is refused, in percent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_pct: Option<u32>,
    /// Percentages of the limit that send `entitlement.usage_threshold_reached`.
    pub thresholds: Vec<u32>,
    /// The event property the limit applies per value of; none for one limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    pub usage: EntitlementUsage,
    /// Switched on, but refused: a blocking limit (with its grace) is reached. Never for a grouped
    /// limit, which refuses per group.
    pub limit_reached: bool,
}

/// On or off.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct BooleanEntitlement {
    pub enabled: bool,
}

/// A setting: a number (as a decimal string), a boolean, a text or a JSON value.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct ConfigEntitlement {
    pub value: serde_json::Value,
}

#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EntitlementValue {
    Boolean(BooleanEntitlement),
    // Not boxed: the discriminator maps each tag to its variant's schema by the field's type.
    Metered(MeteredEntitlement),
    Config(ConfigEntitlement),
}

/// What the customer may use of one feature.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CustomerEntitlement {
    pub feature: FeatureRef,
    /// The feature's name.
    pub name: String,
    /// The product the feature belongs to; none for a feature of the whole catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductRef>,
    pub value: EntitlementValue,
}

crate::v2_list!(CustomerEntitlementList, CustomerEntitlement);

#[derive(serde::Deserialize, utoipa::IntoParams, Debug, Clone, Default)]
#[into_params(parameter_in = Query)]
pub struct EntitlementListQuery {
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// The metrics and products the entitlements name, read in one round.
pub(crate) async fn codes(
    state: &AppState,
    tenant_id: TenantId,
    entitlements: &[EffectiveEntitlement],
) -> Result<Refs, Problem> {
    let mut ids = ReferenceIds::default();
    for entitlement in entitlements {
        if let EffectiveEntitlementValue::Metered { metric_id, .. } = &entitlement.value {
            ids.metrics.push(*metric_id);
        }
        ids.products.extend(
            entitlement
                .feature
                .product
                .as_ref()
                .map(|product| product.id),
        );
    }
    Refs::load(&state.store, tenant_id, &ids).await
}

fn config_value(value: ConfigValue) -> serde_json::Value {
    match value {
        ConfigValue::Number(number) => serde_json::Value::String(number.to_string()),
        ConfigValue::Boolean(flag) => serde_json::Value::Bool(flag),
        ConfigValue::Text(text) => serde_json::Value::String(text),
        ConfigValue::Json(json) => json,
    }
}

pub(crate) fn to_api(entitlement: EffectiveEntitlement, refs: &Refs) -> CustomerEntitlement {
    let value = match entitlement.value {
        EffectiveEntitlementValue::Boolean { enabled } => {
            EntitlementValue::Boolean(BooleanEntitlement { enabled })
        }
        EffectiveEntitlementValue::Metered {
            metric_id,
            limit,
            reset_period,
            overage_behavior,
            thresholds,
            enabled,
            group_by,
            usage,
            limit_reached,
        } => {
            let (at_limit, grace_pct) = match overage_behavior {
                OverageBehavior::Block { grace_period_pct } => (AtLimit::Block, grace_period_pct),
                OverageBehavior::Allow => (AtLimit::Allow, None),
            };
            EntitlementValue::Metered(MeteredEntitlement {
                enabled,
                metric: refs.metric(metric_id),
                limit,
                reset: reset_period.into(),
                at_limit,
                grace_pct,
                thresholds,
                group_by,
                usage: EntitlementUsage {
                    consumed: usage.consumed,
                    remaining: usage.remaining,
                    period_start: usage.period_start.map(Into::into),
                    reset_at: usage.reset_at.map(Into::into),
                },
                limit_reached,
            })
        }
        EffectiveEntitlementValue::Config { value } => {
            EntitlementValue::Config(ConfigEntitlement {
                value: config_value(value),
            })
        }
    };
    let feature = entitlement.feature;
    CustomerEntitlement {
        feature: FeatureRef {
            id: feature.id,
            code: feature.code,
        },
        name: feature.name,
        product: feature.product.map(|product| refs.product(product.id)),
        value,
    }
}

/// List a customer's entitlements
///
/// Every feature the customer is entitled to, its value merged from what the customer holds
/// (subscriptions, add-ons, overrides) over the feature's default, in feature order.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements",
    operation_id = "list_customer_entitlements",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        EntitlementListQuery,
    ),
    responses(
        (status = 200, description = "A page of the customer's entitlements", body = CustomerEntitlementList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_entitlements(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_alias): Path<String>,
    Query(query): Query<EntitlementListQuery>,
) -> Result<Json<CustomerEntitlementList>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let entitlements = state
        .services
        .get_effective_entitlements(customer.id, auth.tenant_id)
        .await?;
    let paged = page(
        entitlements,
        |entitlement| entitlement.feature.id.as_uuid(),
        &PageQuery {
            limit: query.limit,
            after: query.after,
        },
    )?;
    let refs = codes(&state, auth.tenant_id, &paged.data).await?;
    Ok(Json(
        paged.map(|entitlement| to_api(entitlement, &refs)).into(),
    ))
}

/// Get a customer's entitlement to a feature
///
/// The feature by code or id. A feature the customer is not entitled to is not found.
#[utoipa::path(
    get,
    tag = "Customers",
    path = "/api/v2/customers/{id_or_alias}/entitlements/{feature}",
    operation_id = "get_customer_entitlement",
    params(
        ("id_or_alias" = String, Path, description = "Customer id, or its alias as `ext:<alias>`"),
        ("feature" = String, Path, description = "Feature code or id"),
    ),
    responses(
        (status = 200, description = "The entitlement", body = CustomerEntitlement),
        (status = 404, description = "The customer, the feature, or the customer's entitlement to it is not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_entitlement(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_alias, feature)): Path<(String, String)>,
) -> Result<Json<CustomerEntitlement>, Problem> {
    let customer = ids::customer(&state, auth.tenant_id, &id_or_alias).await?;
    let raw = feature.trim();
    let id_or_code = match FeatureId::parse_uuid(raw) {
        Ok(id) => AliasOr::Id(id),
        Err(_) => AliasOr::from_str(raw).unwrap_or_else(|never| match never {}),
    };
    let feature = state
        .store
        .get_feature_by_id_or_code(id_or_code, auth.tenant_id)
        .await?;
    let entitlement = state
        .services
        .get_effective_entitlement_for_feature(customer.id, auth.tenant_id, feature.id)
        .await?
        .ok_or_else(|| {
            Problem::not_found(format!(
                "The customer is not entitled to the feature '{}'",
                feature.code
            ))
        })?;
    let entitlements = [entitlement];
    let refs = codes(&state, auth.tenant_id, &entitlements).await?;
    let [entitlement] = entitlements;
    Ok(Json(to_api(entitlement, &refs)))
}
