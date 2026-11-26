//! A subscription's readout on v2 (ADR 0011 §3.7): each feature at the level it includes it,
//! with the deciding subscription's value there, and its own grant when another one decides.
use super::resolve::{UsageReading, resolve_customer, resolve_resource_input};
use super::snapshot::LevelRead;
use super::tiers::any_resource;
use super::winning;
use crate::domain::entitlement_checks::{
    LevelUsage, LevelValue, ResolvedFeature, ResolvedLevel, SnapshotLevel,
};
use crate::domain::entitlement_fold::Contribution;
use crate::domain::entitlements::{Feature, SubscriptionGrant};
use crate::domain::resources::{ResolvedResource, ResourceInput, ResourceType};
use crate::repositories::entitlements::SourceNames;
use crate::repositories::resources::load_types;
use crate::repositories::subscriptions::SubscriptionInterface;
use crate::services::credit::forecast::plan_names_tx;
use crate::store::PgConn;
use crate::{Store, StoreResult};
use chrono::{DateTime, Utc};
use common_domain::ids::{
    CustomerId, FeatureId, ResourceId, ResourceTypeId, SubscriptionId, TenantId,
};
use diesel_models::resources::ResourceRow;
use itertools::Itertools;
use std::collections::{BTreeMap, HashMap};

/// A feature on a subscription's readout, at the level its value is read on.
pub(crate) struct IncludedLevel {
    pub feature: ResolvedFeature,
    pub level: ResolvedLevel,
    /// The level's resource, to count its usage; none at the customer level, or for a type
    /// level read on no resource in particular.
    pub resource: Option<(ResolvedResource, ResourceType)>,
    pub own_grant: Option<SubscriptionGrant>,
}

/// The level of `feature` the subscription is a candidate on, the feature's own first; else,
/// when no subscription includes it at all, its answering level (defaults and overrides).
fn level_of(feature: &ResolvedFeature, subscription_id: SubscriptionId) -> Option<&ResolvedLevel> {
    let valued = || {
        feature
            .levels
            .iter()
            .filter(|l| matches!(l.value, LevelValue::Value(_)))
    };
    valued()
        .find(|l| {
            l.candidates
                .iter()
                .any(|c| c.subscription_id == subscription_id)
        })
        .or_else(|| {
            valued().find(|l| l.own && l.deciding_subscription.is_none() && l.candidates.is_empty())
        })
}

/// The subscription read: the customer's, or one stored resource's.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SubscriptionOf {
    pub id: SubscriptionId,
    pub customer_id: CustomerId,
    pub resource_id: Option<ResourceId>,
}

/// The subscription's resource as a check naming it resolves it.
async fn resource_of(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription: SubscriptionOf,
) -> StoreResult<Option<(ResolvedResource, ResourceType)>> {
    let Some(id) = subscription.resource_id else {
        return Ok(None);
    };
    let Some(row) = ResourceRow::find_by_id(conn, tenant_id, id).await? else {
        return Ok(None);
    };
    let types = load_types(conn, tenant_id).await?;
    let Some(key) = types
        .iter()
        .find(|t| t.id == row.resource_type_id)
        .and_then(|t| t.keys.first())
    else {
        return Ok(None);
    };
    let input = ResourceInput {
        resources: BTreeMap::from([(key.clone(), row.key_value)]),
        attributes: BTreeMap::new(),
    };
    resolve_resource_input(conn, tenant_id, subscription.customer_id, &input).await
}

/// Every feature the subscription includes, read on its resource (a per-site feature of a
/// customer subscription on a site without one of its own), plus those no subscription includes.
pub(crate) async fn included_levels(
    conn: &mut PgConn,
    tenant_id: TenantId,
    subscription: SubscriptionOf,
    now: DateTime<Utc>,
) -> StoreResult<Vec<IncludedLevel>> {
    let customer_id = subscription.customer_id;
    let resource = resource_of(conn, tenant_id, subscription).await?;
    let resolved = resolve_customer(
        conn,
        tenant_id,
        customer_id,
        resource.as_ref().map(|(r, _)| r),
        None,
        now,
    )
    .await?;
    let mut picked: Vec<(ResolvedFeature, ResolvedLevel, bool)> = Vec::new();
    let mut elsewhere: HashMap<ResourceTypeId, Vec<FeatureId>> = HashMap::new();
    for feature in resolved.features {
        if let Some(level) = level_of(&feature, subscription.id).cloned() {
            let on_resource = level.level.type_id().is_some();
            picked.push((feature, level, on_resource));
        } else if let Some(type_id) = feature.feature.level_type_id
            && resource.as_ref().is_none_or(|(r, _)| r.type_id != type_id)
        {
            elsewhere
                .entry(type_id)
                .or_default()
                .push(feature.feature.id);
        }
    }
    if !elsewhere.is_empty() {
        let types = load_types(conn, tenant_id).await?;
        for (type_id, features) in elsewhere {
            let Some(resource_type) = types.iter().find(|t| t.id == type_id) else {
                continue;
            };
            let any = any_resource(resource_type);
            let on_any = resolve_customer(
                conn,
                tenant_id,
                customer_id,
                Some(&any),
                Some(&features),
                now,
            )
            .await?;
            for feature in on_any.features {
                if let Some(level) = level_of(&feature, subscription.id).cloned() {
                    picked.push((feature, level, false));
                }
            }
        }
    }

    let deciders: Vec<SubscriptionId> = picked
        .iter()
        .filter_map(|(_, level, _)| level.deciding_subscription)
        .filter(|id| *id != subscription.id)
        .unique()
        .collect();
    let plans = plan_names_tx(conn, tenant_id, &deciders).await?;
    Ok(picked
        .into_iter()
        .map(|(feature, level, on_resource)| {
            let own_grant = level
                .deciding_subscription
                .filter(|id| *id != subscription.id)
                .map(|deciding| {
                    let candidate = |id| {
                        level
                            .candidates
                            .iter()
                            .find(move |c| c.subscription_id == id)
                    };
                    SubscriptionGrant {
                        value: candidate(subscription.id).and_then(|c| c.value.clone()),
                        deciding_subscription_id: deciding,
                        deciding_plan_name: plans.get(&deciding).cloned(),
                        deciding_resource_path: candidate(deciding)
                            .and_then(|c| c.resource_path.clone()),
                    }
                });
            IncludedLevel {
                resource: resource.clone().filter(|_| on_resource),
                feature,
                level,
                own_grant,
            }
        })
        .collect())
}

/// A feature the subscription includes, at the level it includes it: that level's deciding
/// value, policy, window and usage (a count your app reports included), the row it comes from,
/// and what this subscription grants there when another one decides.
#[derive(Debug, Clone)]
pub struct SubscriptionEntitlementLevel {
    pub feature: Feature,
    pub level: SnapshotLevel,
    pub own_grant: Option<SubscriptionGrant>,
}

impl Store {
    /// The subscription's readout by level (ADR 0011 §3.7), then the features no subscription
    /// includes; each level's usage read once.
    pub async fn subscription_entitlement_levels(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Vec<SubscriptionEntitlementLevel>> {
        let sub = self.get_subscription(tenant_id, subscription_id).await?;
        let now = common_clock::now();
        let of = SubscriptionOf {
            id: subscription_id,
            customer_id: sub.customer_id,
            resource_id: sub.resource_id,
        };
        let included = {
            let mut conn = self.get_conn().await?;
            included_levels(&mut conn, tenant_id, of, now).await?
        };
        let reads: Vec<LevelRead> = included
            .iter()
            .map(|i| (&i.feature.feature, &i.level, i.resource.as_ref()))
            .collect();
        let usages = self
            .read_usages(
                tenant_id,
                sub.customer_id,
                &reads,
                now,
                UsageReading::readout(),
            )
            .await?;
        let stored = |i: &IncludedLevel| {
            i.resource
                .as_ref()
                .and_then(|(r, _)| r.reference.stored_id())
        };
        let names = {
            let mut conn = self.get_conn().await?;
            SourceNames::for_contributions(
                &mut conn,
                tenant_id,
                included.iter().filter_map(|i| winning(&i.level.explain)),
                included.iter().filter_map(stored).unique(),
            )
            .await?
        };
        Ok(included
            .into_iter()
            .zip(usages)
            .map(|(included, used)| {
                let resource = stored(&included);
                let level = included.level;
                let origin = winning(&level.explain).map(|c| Contribution {
                    source_name: names.contribution(c, resource),
                    ..c.clone()
                });
                SubscriptionEntitlementLevel {
                    feature: included.feature.feature,
                    level: SnapshotLevel {
                        level: level.level,
                        path: level.path,
                        value: level.value,
                        policy: level.policy,
                        deciding_subscription: level.deciding_subscription,
                        window: level.window,
                        used: used.used(),
                        usage_unavailable: used == LevelUsage::Unavailable,
                        origin,
                    },
                    own_grant: included.own_grant,
                }
            })
            .collect())
    }
}
