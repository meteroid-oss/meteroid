//! The snapshot's etag (ADR 0011 §3.10): a hash of everything the snapshot answers but usage.
//! Computed on read.
use crate::domain::entitlement_checks::FeatureScope;
use crate::domain::entitlement_fold::{EvalLevel, FoldRow};
use crate::domain::entitlement_levels::LevelValue;
use crate::domain::entitlement_policy::FeaturePolicy;
use chrono::{DateTime, Utc};
use common_domain::ids::{FeatureId, ProductId, SubscriptionId};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct LevelFingerprint {
    pub feature_id: FeatureId,
    pub level: EvalLevel,
    pub path: String,
    /// Not entitled, a resource required and not subscribed differ here.
    pub value: LevelValue,
    pub policy: FeaturePolicy,
    pub deciding_subscription: Option<SubscriptionId>,
    /// The window's end; none for a sliding window, which ends at every read.
    pub reset_at: Option<DateTime<Utc>>,
    /// The winning row's lineage, quantity, units held (per-unit limits) and display name.
    pub origin: Option<(String, u32, Option<u32>, Option<String>)>,
}

/// What a snapshot names of a feature.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct FeatureFingerprint {
    pub id: FeatureId,
    pub name: String,
    pub code: String,
    pub unit: Option<String>,
    pub product: Option<(ProductId, String)>,
    pub scope: FeatureScope,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Fingerprint {
    pub features: Vec<FeatureFingerprint>,
    pub levels: Vec<LevelFingerprint>,
    pub attributes_ignored: Vec<String>,
    /// The next `valid_from` or `valid_until` ahead: crossing it changes the etag.
    pub next_change: Option<DateTime<Utc>>,
}

/// The same for the same values, whatever order the features and levels come in.
pub fn etag(fingerprint: &Fingerprint) -> String {
    let mut levels: Vec<&LevelFingerprint> = fingerprint.levels.iter().collect();
    levels.sort_by_key(|l| (*l.feature_id, l.level.type_id().map(|t| *t)));
    let mut features: Vec<&FeatureFingerprint> = fingerprint.features.iter().collect();
    features.sort_by_key(|f| *f.id);
    let json = serde_json::to_vec(&(
        features,
        levels,
        &fingerprint.attributes_ignored,
        fingerprint.next_change,
    ))
    .expect("a fingerprint serialises");
    hex::encode(&Sha256::digest(json)[..16])
}

/// The earliest validity bound after `now` among the rows.
pub fn next_change(rows: &[FoldRow], now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    rows.iter()
        .flat_map(|r| [r.valid_from, r.valid_until])
        .flatten()
        .filter(|bound| *bound > now)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entitlement_fold::{Effect, GrantValue, LimitValue, Tier};
    use crate::domain::entitlement_policy::AtLimit;
    use chrono::{Duration, TimeZone};
    use common_domain::ids::{BaseId, EntitlementEntityId, EntitlementId, ResourceTypeId};
    use rust_decimal::Decimal;

    fn level(feature_id: FeatureId, level: EvalLevel, limit: i64) -> LevelFingerprint {
        LevelFingerprint {
            feature_id,
            level,
            path: String::new(),
            value: LevelValue::Value(GrantValue::Limit(LimitValue::Amount(Decimal::from(limit)))),
            policy: FeaturePolicy::default(),
            deciding_subscription: None,
            reset_at: None,
            origin: None,
        }
    }

    #[test]
    fn equal_values_give_equal_etags() {
        let (api_calls, projects) = (FeatureId::new(), FeatureId::new());
        let site = EvalLevel::Type(ResourceTypeId::new());
        let a = Fingerprint {
            features: vec![FeatureFingerprint {
                id: api_calls,
                name: "API calls".into(),
                code: "api_calls".into(),
                unit: None,
                product: None,
                scope: FeatureScope::Customer,
            }],
            levels: vec![
                level(api_calls, EvalLevel::Customer, 1000),
                level(projects, site, 5),
                level(projects, EvalLevel::Customer, 50),
            ],
            attributes_ignored: vec![],
            next_change: None,
        };
        let mut reordered = a.clone();
        reordered.levels.reverse();
        assert_eq!(etag(&a), etag(&reordered));
        assert_eq!(etag(&a), etag(&a.clone()));
        assert_eq!(etag(&a).len(), 32);

        let changes: [fn(&mut Fingerprint); 8] = [
            |f| f.levels[1].value = LevelValue::Value(GrantValue::Limit(LimitValue::Unlimited)),
            |f| f.levels[1].value = LevelValue::ResourceRequired,
            |f| f.levels[1].value = LevelValue::NotSubscribed,
            |f| f.features[0].name = "Calls".into(),
            |f| f.features[0].scope = FeatureScope::Resource,
            |f| f.levels[0].policy.at_limit = AtLimit::AllowAndBill,
            |f| f.levels[0].policy.thresholds = vec![80],
            |f| f.next_change = Some(common_clock::now()),
        ];
        for change in changes {
            let mut changed = a.clone();
            change(&mut changed);
            assert_ne!(etag(&a), etag(&changed), "{changed:?}");
        }
    }

    #[test]
    fn the_next_change_is_the_earliest_bound_ahead() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let at = |d| t0 + Duration::days(d);
        let row = |valid_from, valid_until| FoldRow {
            lineage_id: EntitlementId::new(),
            lineage_created_at: t0,
            lineage_anchor: t0,
            source: EntitlementEntityId::Feature(FeatureId::new()),
            tier: Tier::Default,
            effect: Effect::Set,
            value: GrantValue::Boolean(true),
            condition: None,
            condition_met: true,
            quantity: 1,
            units: None,
            policy_override: None,
            created_at: t0,
            valid_from,
            valid_until,
            revoked_at: None,
        };
        let rows = [
            row(Some(at(1)), Some(at(40))),
            row(Some(at(20)), None),
            row(None, Some(at(30))),
        ];
        assert_eq!(next_change(&rows, at(10)), Some(at(20)));
        assert_eq!(next_change(&rows, at(20)), Some(at(30)));
        assert_eq!(next_change(&rows, at(40)), None);
    }
}
