//! The snapshot (ADR 0011 §3.10, §3.11): every feature at every level with its read usage, and an
//! etag over all of it but usage, computed on read.
use crate::domain::entitlement_checks::{
    LevelUsage, LevelValue, ResolvedCustomer, ResolvedFeature, ResolvedLevel, ResourceSnapshot,
    Snapshot, SnapshotFeature, SnapshotLevel, SnapshotRead, usage_warnings,
};
use crate::domain::entitlement_etag::{FeatureFingerprint, Fingerprint, LevelFingerprint, etag};
use crate::domain::entitlement_fold::{Contribution, EvalLevel, Tier};
use crate::domain::entitlement_levels::{LevelOutcome, combined_value};
use crate::domain::entitlements::{Feature, FeatureKind, LimitCountedBy, ResetPeriod};
use crate::domain::resources::{
    Attributes, ResolvedResource, Resource, ResourceInput, ResourceType, resolve_input,
};
use crate::errors::StoreError;
use crate::repositories::entitlements::SourceNames;
use crate::repositories::resources::load_types;
use crate::services::entitlements::limits::load_metrics;
use crate::services::entitlements::resolve::{
    CustomerRows, UsageReading, resolve_feature, resolve_resource_input,
};
use crate::services::entitlements::winning;
use crate::{Store, StoreResult};
use chrono::{DateTime, Utc};
use common_domain::ids::{CustomerId, FeatureId, ResourceId, TenantId};
use diesel_models::resources::ResourceRow;
use error_stack::{Report, bail};
use futures::{StreamExt, TryStreamExt, stream};
use itertools::Itertools;
use std::collections::{BTreeMap, HashMap};

pub fn fingerprint(
    resolved: &ResolvedCustomer,
    attributes_ignored: &[String],
    names: &SourceNames,
    resource: Option<ResourceId>,
) -> Fingerprint {
    let features = resolved.features.iter().map(|f| FeatureFingerprint {
        id: f.feature.id,
        name: f.feature.name.clone(),
        code: f.feature.code.clone(),
        unit: f.feature.unit.clone(),
        product: f.feature.product.as_ref().map(|p| (p.id, p.name.clone())),
        scope: f.scope,
    });
    let levels = resolved.features.iter().flat_map(|f| {
        f.levels.iter().map(|l| LevelFingerprint {
            feature_id: f.feature.id,
            level: l.level,
            path: l.path.clone(),
            value: l.value.clone(),
            policy: l.policy.clone(),
            deciding_subscription: l.deciding_subscription,
            reset_at: l
                .window
                .as_ref()
                .filter(|w| !matches!(w.reset_period, ResetPeriod::SlidingWindow { .. }))
                .and_then(|w| w.end),
            origin: winning(&l.explain).map(|c| {
                (
                    c.lineage_id.to_string(),
                    c.quantity,
                    c.units,
                    names.contribution(c, resource),
                )
            }),
        })
    });
    Fingerprint {
        features: features.collect(),
        levels: levels.collect(),
        attributes_ignored: attributes_ignored.to_vec(),
        next_change: resolved
            .features
            .iter()
            .flat_map(|f| f.levels.iter().filter_map(|l| l.next_change))
            .min(),
    }
}

/// `If-None-Match`: a list of tags, weak or strong, or `*`.
fn matches(if_none_match: &str, etag: &str) -> bool {
    if_none_match
        .split(',')
        .map(str::trim)
        .any(|tag| tag == "*" || tag.trim_start_matches("W/").trim_matches('"') == etag)
}

/// Metering reads a snapshot runs at once.
const USAGE_READS: usize = 8;

/// Resources one batched snapshot reads.
pub const MAX_SNAPSHOT_RESOURCES: usize = 100;

/// What a level's usage depends on: levels sharing it, as the customer's level across
/// resources, are read once.
type UsageKey = (
    FeatureId,
    EvalLevel,
    String,
    Option<(DateTime<Utc>, Option<DateTime<Utc>>)>,
);

pub(crate) type LevelRead<'a> = (
    &'a Feature,
    &'a ResolvedLevel,
    Option<&'a (ResolvedResource, ResourceType)>,
);

impl Store {
    pub async fn entitlement_snapshot(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resources: &ResourceInput,
        if_none_match: Option<&str>,
    ) -> StoreResult<SnapshotRead> {
        let now = common_clock::now();
        let (resolved, resource) = {
            let mut conn = self.get_conn().await?;
            let resource =
                resolve_resource_input(&mut conn, tenant_id, customer_id, resources).await?;
            let resolved = self
                .resolve_customer_entitlements(
                    &mut conn,
                    tenant_id,
                    customer_id,
                    resource.as_ref().map(|(r, _)| r),
                    None,
                    now,
                )
                .await?;
            (resolved, resource)
        };
        let ignored = resource
            .as_ref()
            .map(|(r, _)| r.attributes_ignored.clone())
            .unwrap_or_default();
        let resource_id = resource.as_ref().and_then(|(r, _)| r.reference.stored_id());
        // Names are part of the etag, so a renamed plan or add-on invalidates cached origins.
        let names = {
            let mut conn = self.get_conn().await?;
            SourceNames::for_contributions(
                &mut conn,
                tenant_id,
                winners(&resolved.features),
                resource_id,
            )
            .await?
        };
        let etag = etag(&fingerprint(&resolved, &ignored, &names, resource_id));
        if if_none_match.is_some_and(|tag| matches(tag, &etag)) {
            return Ok(SnapshotRead::NotModified { etag });
        }

        let read_for = resource.as_ref();
        let reads: Vec<LevelRead> = resolved
            .features
            .iter()
            .flat_map(|f| f.levels.iter().map(move |l| (&f.feature, l, read_for)))
            .collect();
        let usages = self
            .read_usages(
                tenant_id,
                customer_id,
                &reads,
                now,
                UsageReading::snapshot(),
            )
            .await?;
        let features = snapshot_features(
            resolved.features,
            &mut usages.into_iter(),
            &names,
            resource_id,
        );
        Ok(SnapshotRead::Modified(Snapshot {
            etag,
            warnings: usage_warnings(&features),
            features,
            attributes_ignored: ignored,
        }))
    }

    /// The snapshot's features among `features` only, without an etag: one feature's read
    /// resolves no other.
    pub async fn entitlement_snapshot_features(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resources: &ResourceInput,
        features: &[FeatureId],
    ) -> StoreResult<Vec<SnapshotFeature>> {
        if features.is_empty() {
            return Ok(vec![]);
        }
        let now = common_clock::now();
        let (resolved, resource) = {
            let mut conn = self.get_conn().await?;
            let resource =
                resolve_resource_input(&mut conn, tenant_id, customer_id, resources).await?;
            let resolved = self
                .resolve_customer_entitlements(
                    &mut conn,
                    tenant_id,
                    customer_id,
                    resource.as_ref().map(|(r, _)| r),
                    Some(features),
                    now,
                )
                .await?;
            (resolved, resource)
        };
        let resource_id = resource.as_ref().and_then(|(r, _)| r.reference.stored_id());
        let names = {
            let mut conn = self.get_conn().await?;
            SourceNames::for_contributions(
                &mut conn,
                tenant_id,
                winners(&resolved.features),
                resource_id,
            )
            .await?
        };
        let read_for = resource.as_ref();
        let reads: Vec<LevelRead> = resolved
            .features
            .iter()
            .flat_map(|f| f.levels.iter().map(move |l| (&f.feature, l, read_for)))
            .collect();
        let usages = self
            .read_usages(
                tenant_id,
                customer_id,
                &reads,
                now,
                UsageReading::snapshot(),
            )
            .await?;
        Ok(snapshot_features(
            resolved.features,
            &mut usages.into_iter(),
            &names,
            resource_id,
        ))
    }

    /// The snapshots of a customer's stored resources (or only of `features`), resolved over one
    /// load of the customer's rows, each distinct usage read once.
    pub async fn entitlement_snapshots(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        resource_ids: &[ResourceId],
        features: Option<&[FeatureId]>,
    ) -> StoreResult<Vec<ResourceSnapshot>> {
        let ids: Vec<ResourceId> = resource_ids.iter().copied().unique().collect();
        if ids.len() > MAX_SNAPSHOT_RESOURCES {
            bail!(StoreError::InvalidArgument(format!(
                "at most {MAX_SNAPSHOT_RESOURCES} resources per request"
            )));
        }
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let now = common_clock::now();
        let (data, resources) = {
            let mut conn = self.get_conn().await?;
            let stored = ResourceRow::find_by_ids(&mut conn, tenant_id, &ids)
                .await?
                .into_iter()
                .map(Resource::try_from)
                .collect::<StoreResult<Vec<_>>>()?;
            let types = load_types(&mut conn, tenant_id).await?;
            let resources = ids
                .iter()
                .map(|id| {
                    let resource = stored
                        .iter()
                        .find(|r| r.id == *id && r.customer_id == customer_id)
                        .ok_or_else(|| not_found(format!("resource {id}")))?;
                    stored_resource(&types, resource)
                })
                .collect::<StoreResult<Vec<_>>>()?;
            let data = CustomerRows::load(&mut conn, tenant_id, customer_id, features, now).await?;
            (data, resources)
        };

        let resolved: Vec<(ResourceId, Vec<ResolvedFeature>)> = ids
            .iter()
            .zip(&resources)
            .map(|(id, (resource, _))| {
                let features = data
                    .features
                    .iter()
                    .filter_map(|f| resolve_feature(f, &data, Some(resource), now))
                    .collect();
                (*id, features)
            })
            .collect();
        let reads: Vec<LevelRead> = resolved
            .iter()
            .zip(&resources)
            .flat_map(|((_, features), resource)| {
                features.iter().flat_map(move |f| {
                    f.levels
                        .iter()
                        .map(move |l| (&f.feature, l, Some(resource)))
                })
            })
            .collect();
        let usages = self
            .read_usages(
                tenant_id,
                customer_id,
                &reads,
                now,
                UsageReading::snapshot(),
            )
            .await?;
        let overridden = resolved
            .iter()
            .filter(|(_, features)| winners(features).any(|c| c.tier == Tier::ResourceOverride))
            .map(|(id, _)| *id);
        let names = {
            let mut conn = self.get_conn().await?;
            SourceNames::for_contributions(
                &mut conn,
                tenant_id,
                resolved.iter().flat_map(|(_, features)| winners(features)),
                overridden,
            )
            .await?
        };
        let mut usages = usages.into_iter();
        Ok(resolved
            .into_iter()
            .map(|(resource_id, features)| {
                let features = snapshot_features(features, &mut usages, &names, Some(resource_id));
                ResourceSnapshot {
                    resource_id,
                    warnings: usage_warnings(&features),
                    features,
                }
            })
            .collect())
    }

    /// Each level's usage, in order; levels with the same [`UsageKey`] share one metering read.
    /// A level metering cannot answer for is unavailable, the others are read.
    pub(crate) async fn read_usages(
        &self,
        tenant_id: TenantId,
        customer_id: CustomerId,
        levels: &[LevelRead<'_>],
        now: DateTime<Utc>,
        reading: UsageReading,
    ) -> StoreResult<Vec<LevelUsage>> {
        let metric_ids: Vec<_> = levels
            .iter()
            .filter_map(|(f, _, _)| match f.kind {
                FeatureKind::Limit(LimitCountedBy::Events(id)) => Some(id),
                _ => None,
            })
            .unique()
            .collect();
        let metrics = if metric_ids.is_empty() {
            HashMap::new()
        } else {
            let mut conn = self.get_conn().await?;
            load_metrics(&mut conn, &metric_ids, tenant_id).await?
        };

        let mut distinct: HashMap<UsageKey, usize> = HashMap::new();
        let mut unique: Vec<&LevelRead> = Vec::new();
        let slots: Vec<Option<usize>> = levels
            .iter()
            .map(|read| {
                let (feature, level, _) = read;
                if !matches!(level.value, LevelValue::Value(_)) {
                    return None;
                }
                let key = (
                    feature.id,
                    level.level,
                    level.path.clone(),
                    level.window.as_ref().map(|w| (w.start, w.end)),
                );
                Some(*distinct.entry(key).or_insert_with(|| {
                    unique.push(read);
                    unique.len() - 1
                }))
            })
            .collect();

        // Futures collected first: a stream `.map` closure borrowing the levels makes the
        // caller's future fail the higher-ranked `Send` check when spawned.
        let fetches: Vec<_> = unique
            .into_iter()
            .map(|(feature, level, resource)| {
                self.level_usage_read(
                    tenant_id,
                    customer_id,
                    feature,
                    level,
                    *resource,
                    now,
                    Some(&metrics),
                    &reading,
                )
            })
            .collect();
        let read: Vec<LevelUsage> = stream::iter(fetches)
            .buffered(USAGE_READS)
            .try_collect()
            .await?;
        Ok(slots
            .into_iter()
            .map(|slot| slot.map_or(LevelUsage::Read(None), |i| read[i]))
            .collect())
    }
}

/// A stored resource as a check naming it by its canonical key reads it.
fn stored_resource(
    types: &[ResourceType],
    resource: &Resource,
) -> StoreResult<(ResolvedResource, ResourceType)> {
    let resource_type = types
        .iter()
        .find(|t| t.id == resource.resource_type_id)
        .ok_or_else(|| not_found(format!("resource type {}", resource.resource_type_id)))?;
    let key = resource_type
        .keys
        .first()
        .ok_or_else(|| not_found(format!("key of resource type {}", resource_type.code)))?;
    let input = ResourceInput {
        resources: BTreeMap::from([(key.clone(), resource.key_value.clone())]),
        attributes: Attributes::new(),
    };
    let resolved = resolve_input(
        std::slice::from_ref(resource_type),
        std::slice::from_ref(resource),
        &input,
    )?
    .ok_or_else(|| not_found(format!("resource {}", resource.id)))?;
    Ok((resolved, resource_type.clone()))
}

fn not_found(what: String) -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound(what))
}

pub(crate) fn winners(features: &[ResolvedFeature]) -> impl Iterator<Item = &Contribution> {
    features
        .iter()
        .flat_map(|f| f.levels.iter().filter_map(|l| winning(&l.explain)))
}

/// `usages` holds each level's, in the order of `features`.
fn snapshot_features(
    features: Vec<ResolvedFeature>,
    usages: &mut impl Iterator<Item = LevelUsage>,
    names: &SourceNames,
    resource: Option<ResourceId>,
) -> Vec<SnapshotFeature> {
    let mut snapshot = Vec::with_capacity(features.len());
    for resolved in features {
        let mut levels = Vec::with_capacity(resolved.levels.len());
        let mut outcomes = Vec::with_capacity(resolved.levels.len());
        for level in resolved.levels {
            let usage = usages.next().unwrap_or(LevelUsage::Read(None));
            let used = usage.used();
            outcomes.push(LevelOutcome {
                level: level.level,
                own: level.own,
                value: level.value.clone(),
                policy: level.policy.clone(),
                usage,
                reset_at: level.window.as_ref().and_then(|w| w.end),
            });
            let origin = winning(&level.explain).map(|c| Contribution {
                source_name: names.contribution(c, resource),
                ..c.clone()
            });
            levels.push(SnapshotLevel {
                level: level.level,
                path: level.path,
                value: level.value,
                policy: level.policy,
                deciding_subscription: level.deciding_subscription,
                window: level.window,
                used,
                usage_unavailable: usage == LevelUsage::Unavailable,
                origin,
            });
        }
        snapshot.push(SnapshotFeature {
            value: combined_value(&resolved.feature.value_order(), &outcomes),
            feature: resolved.feature,
            scope: resolved.scope,
            levels,
        });
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn if_none_match_takes_lists_weak_tags_and_a_wildcard() {
        assert!(matches("\"abc\"", "abc"));
        assert!(matches("W/\"abc\", \"def\"", "def"));
        assert!(matches("*", "abc"));
        assert!(!matches("\"abd\"", "abc"));
    }
}
