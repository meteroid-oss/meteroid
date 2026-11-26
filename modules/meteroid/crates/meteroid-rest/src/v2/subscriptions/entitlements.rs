//! `GET /api/v2/subscriptions/{id_or_external_id}/entitlements[/{feature}]` (ADR-0013 §25): what
//! one subscription grants, each feature it includes with its level's deciding value, policy and
//! usage (counts your app reports included), and its own grant when another subscription decides
//! (ADR-0011 §3.7).
use crate::AppState;
use crate::v2::entitlements::model::{
    Codes, EntitlementValue, FeatureKind as ApiFeatureKind, LevelKind, LevelResource, Origin,
    Policy, UsageWindow, level_kind, level_resource,
};
use crate::v2::extract::Path;
use crate::v2::ids;
use crate::v2::problem::Problem;
use crate::v2::refs::{FeatureRef, ProductRef, ResourceTypeRef, SubscriptionRef};
use axum::Extension;
use axum::Json;
use axum::extract::State;
use common_domain::ids::{FeatureId, SubscriptionId, TenantId};
use common_grpc::middleware::server::auth::AuthorizedAsTenant;
use meteroid_store::domain::entitlement_checks::LevelValue;
use meteroid_store::domain::entitlements::{FeatureKind, LimitCountedBy, SubscriptionGrant};
use meteroid_store::services::entitlements::subscription::SubscriptionEntitlementLevel;
use rust_decimal::Decimal;
use serde::Serialize;
use utoipa::ToSchema;

/// What this subscription's own rows grant on a level another subscription decides.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct OwnGrant {
    /// Absent when it includes the feature through its product alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    pub deciding_subscription: SubscriptionRef,
    /// The name of the deciding subscription's plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_plan_name: Option<String>,
    /// The resource the deciding subscription is for; absent for a subscription of the customer
    /// as a whole.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_resource: Option<LevelResource>,
}

/// A feature the subscription includes, at the level it includes it.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct SubscriptionEntitlement {
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
    pub level: LevelKind,
    /// With `level: resource_type`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<ResourceTypeRef>,
    /// With `level: resource_type`: the resource the subscription is for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<LevelResource>,
    /// The deciding subscription's value at the level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<EntitlementValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// A limit's usage at the level: the count your app reported, or the metric's events over
    /// `window`; absent when it could not be read.
    #[serde(
        with = "rust_decimal::serde::str_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>, format = "decimal")]
    pub used: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<UsageWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deciding_subscription: Option<SubscriptionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_grant: Option<OwnGrant>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct SubscriptionEntitlementList {
    pub data: Vec<SubscriptionEntitlement>,
}

fn own_grant(grant: SubscriptionGrant, codes: &Codes) -> OwnGrant {
    OwnGrant {
        value: grant.value.as_ref().map(Into::into),
        deciding_subscription: codes.subscription(grant.deciding_subscription_id),
        deciding_plan_name: grant.deciding_plan_name,
        deciding_resource: grant
            .deciding_resource_path
            .as_deref()
            .and_then(level_resource),
    }
}

fn entitlement(read: SubscriptionEntitlementLevel, codes: &Codes) -> SubscriptionEntitlement {
    let SubscriptionEntitlementLevel {
        feature,
        level,
        own_grant: grant,
    } = read;
    let (kind, resource_type) = level_kind(level.level, codes);
    let limit = matches!(feature.kind, FeatureKind::Limit(_));
    let counted_from_events = matches!(feature.kind, FeatureKind::Limit(LimitCountedBy::Events(_)));
    let facts = codes.facts(&feature);
    SubscriptionEntitlement {
        feature: facts.feature,
        name: facts.name,
        kind: facts.kind,
        unit: facts.unit,
        product: facts.product,
        applies_per: facts.applies_per,
        level: kind,
        resource_type,
        resource: level_resource(&level.path),
        value: match &level.value {
            LevelValue::Value(value) => Some(value.into()),
            _ => None,
        },
        policy: limit.then(|| (&level.policy).into()),
        used: level.used.filter(|_| limit).map(|used| used.normalize()),
        window: level
            .window
            .as_ref()
            .filter(|_| counted_from_events)
            .map(Into::into),
        deciding_subscription: level.deciding_subscription.map(|id| codes.subscription(id)),
        origin: level.origin.as_ref().map(Into::into),
        own_grant: grant.map(|grant| own_grant(grant, codes)),
    }
}

/// List what a subscription grants
///
/// Each feature the subscription includes, with the value deciding at its level (another
/// subscription's when it decides, this one's own then in `own_grant`), its policy and its usage,
/// then the features no subscription includes (defaults and overrides). Not paged: a tenant's
/// features are its catalog.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/entitlements",
    operation_id = "list_subscription_entitlements",
    params(("id_or_external_id" = String, Path, description = "Subscription id or external id")),
    responses(
        (status = 200, description = "What the subscription grants", body = SubscriptionEntitlementList),
        (status = 404, description = "Not found", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_subscription_entitlements(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path(id_or_external_id): Path<String>,
) -> Result<Json<SubscriptionEntitlementList>, Problem> {
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    let data = entitlements(&state, auth.tenant_id, id, None).await?;
    Ok(Json(SubscriptionEntitlementList { data }))
}

/// Get what a subscription grants of a feature
///
/// The feature (code or id) as the subscription's list reads it: one entry per level it is
/// included at, sorted by resource. A feature the list does not hold is not found.
#[utoipa::path(
    get,
    tag = "Subscriptions",
    path = "/api/v2/subscriptions/{id_or_external_id}/entitlements/{feature}",
    operation_id = "get_subscription_entitlement",
    params(
        ("id_or_external_id" = String, Path, description = "Subscription id or external id"),
        ("feature" = String, Path, description = "Feature code or id"),
    ),
    responses(
        (status = 200, description = "The feature at each level the subscription includes it", body = SubscriptionEntitlementList),
        (status = 404, description = "The subscription or the feature is not found, or the subscription does not grant it", body = Problem, content_type = "application/problem+json"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_subscription_entitlement(
    Extension(auth): Extension<AuthorizedAsTenant>,
    State(state): State<AppState>,
    Path((id_or_external_id, feature)): Path<(String, String)>,
) -> Result<Json<SubscriptionEntitlementList>, Problem> {
    let id = ids::subscription_id(&state, auth.tenant_id, &id_or_external_id).await?;
    let feature = ids::feature(&state, auth.tenant_id, feature.trim()).await?;
    let data = entitlements(&state, auth.tenant_id, id, Some(feature.id)).await?;
    if data.is_empty() {
        return Err(Problem::not_found(format!(
            "The subscription does not grant the feature '{}'",
            feature.code
        )));
    }
    Ok(Json(SubscriptionEntitlementList { data }))
}

/// The subscription's levels, of one feature when `only` names it, sorted by feature code then
/// resource.
async fn entitlements(
    state: &AppState,
    tenant_id: TenantId,
    id: SubscriptionId,
    only: Option<FeatureId>,
) -> Result<Vec<SubscriptionEntitlement>, Problem> {
    let mut levels = state
        .store
        .subscription_entitlement_levels(tenant_id, id)
        .await?;
    if let Some(only) = only {
        levels.retain(|level| level.feature.id == only);
    }
    let deciding: Vec<SubscriptionId> = levels
        .iter()
        .flat_map(|l| {
            l.level
                .deciding_subscription
                .into_iter()
                .chain(l.own_grant.as_ref().map(|g| g.deciding_subscription_id))
        })
        .collect();
    let codes = Codes::of_features(
        state,
        tenant_id,
        levels.iter().map(|l| &l.feature),
        deciding,
    )
    .await?;
    let mut data: Vec<SubscriptionEntitlement> = levels
        .into_iter()
        .map(|level| entitlement(level, &codes))
        .collect();
    data.sort_by(|a, b| (&a.feature.code, &a.resource).cmp(&(&b.feature.code, &b.resource)));
    Ok(data)
}
