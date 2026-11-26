//! Metering down or late during a check, a snapshot or a readout (ADR 0011 §3.11, §3.14,
//! ADR-0013 §23): a limit that refuses answers `usage_unavailable`, one that bills allows, and the
//! other entries answer; the store's own answer is never an error for it. A limit whose usage
//! cannot be counted as set up is `misconfigured`, never retried.
use crate::data::ids::{CUST_UBER_ID, METRIC_BANDWIDTH, TENANT_ID};
use crate::helpers::credit_recompute::{
    Fixture, execute, fixture_over_events, linear_subscription,
};
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{AliasOr, BaseId, BillableMetricId};
use meteroid_store::domain::entitlement_checks::{
    CheckReason, CheckRequest, CheckResponse, ReasonType, Snapshot, SnapshotLevel, SnapshotRead,
    effective_usage_warnings,
};
use meteroid_store::domain::entitlement_fold::{GrantValue, LimitValue};
use meteroid_store::domain::entitlement_policy::{AtLimit, FeaturePolicy};
use meteroid_store::domain::entitlements::{
    EffectiveEntitlement, EffectiveEntitlementValue, Feature, FeatureKind, FeatureNew,
    LimitCountedBy,
};
use meteroid_store::domain::resources::ResourceInput;
use meteroid_store::repositories::resources::{ResourceTypeNew, ResourceUpsert};
use meteroid_store::repositories::{EntitlementsInterface, ResourcesInterface};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const WARNING: &str = "usage_unavailable";

async fn feature(f: &Fixture, code: &str, kind: FeatureKind, at_limit: AtLimit) -> Feature {
    let (default, policy) = match kind {
        FeatureKind::Boolean => (GrantValue::Boolean(true), None),
        _ => (
            GrantValue::Limit(LimitValue::Amount(Decimal::from(100))),
            Some(FeaturePolicy {
                at_limit,
                ..FeaturePolicy::default()
            }),
        ),
    };
    f.setup
        .store
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: code.into(),
                code: code.into(),
                description: None,
                kind,
                unit: None,
                level_type_id: None,
                policy,
                default: Some(default),
            },
        )
        .await
        .unwrap()
}

async fn check(f: &Fixture, feature: &Feature) -> CheckResponse {
    f.setup
        .store
        .check_entitlement(
            TENANT_ID,
            CUST_UBER_ID,
            AliasOr::Alias(feature.code.clone()),
            CheckRequest::default(),
        )
        .await
        .unwrap()
}

/// The fixture's events cleared: every limit reads 0 while metering answers.
async fn features(f: &Fixture) -> (Feature, Feature, Feature) {
    f.events.events.lock().unwrap().clear();
    let calls = feature(
        f,
        "calls",
        FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
        AtLimit::Deny,
    )
    .await;
    let billed = feature(
        f,
        "billed_calls",
        FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
        AtLimit::AllowAndBill,
    )
    .await;
    let sso = feature(f, "sso", FeatureKind::Boolean, AtLimit::Deny).await;
    (calls, billed, sso)
}

#[tokio::test]
async fn metering_down_refuses_a_denying_limit_as_unavailable_and_lets_the_others_answer() {
    let f = fixture_over_events().await;
    let (calls, billed, sso) = features(&f).await;
    assert!(check(&f, &calls).await.allowed, "metering up");
    f.events.unavailable.store(true, Ordering::SeqCst);

    let refused = check(&f, &calls).await;
    assert!(!refused.allowed, "{refused:?}");
    assert_eq!(refused.reason, Some(CheckReason::UsageUnavailable));
    assert_eq!(refused.reason_type, Some(ReasonType::Unavailable));
    let retry = refused.retry_after.expect("retried shortly");
    let wait = retry - Utc::now();
    assert!(
        wait > chrono::Duration::zero() && wait <= chrono::Duration::seconds(30),
        "{retry}"
    );
    assert_eq!(refused.levels[0].used, None);
    assert_eq!(
        refused.levels[0].denial,
        Some(CheckReason::UsageUnavailable)
    );
    assert_eq!(refused.warnings, vec![WARNING.to_owned()]);

    let allowed = check(&f, &billed).await;
    assert!(allowed.allowed, "{allowed:?}");
    assert_eq!(allowed.reason, None);
    assert_eq!(allowed.levels[0].used, None);
    assert!(!allowed.levels[0].in_overage);
    assert_eq!(allowed.warnings, vec![WARNING.to_owned()]);

    let boolean = check(&f, &sso).await;
    assert!(boolean.allowed, "{boolean:?}");
    assert!(boolean.warnings.is_empty(), "{boolean:?}");
}

#[tokio::test]
async fn metering_late_is_cut_at_the_checks_deadline() {
    let f = fixture_over_events().await;
    let (calls, _, _) = features(&f).await;
    f.events.stalled.store(true, Ordering::SeqCst);

    let started = Instant::now();
    let refused = check(&f, &calls).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(refused.reason, Some(CheckReason::UsageUnavailable));
    assert_eq!(refused.reason_type, Some(ReasonType::Unavailable));
}

async fn snapshot(f: &Fixture) -> Snapshot {
    match f
        .setup
        .store
        .entitlement_snapshot(TENANT_ID, CUST_UBER_ID, &ResourceInput::default(), None)
        .await
        .unwrap()
    {
        SnapshotRead::Modified(snapshot) => snapshot,
        SnapshotRead::NotModified { .. } => panic!("no etag sent"),
    }
}

fn level(s: &Snapshot, feature: &Feature) -> SnapshotLevel {
    s.features
        .iter()
        .find(|sf| sf.feature.id == feature.id)
        .and_then(|sf| sf.levels.first().cloned())
        .unwrap_or_else(|| panic!("{} in the snapshot", feature.code))
}

#[tokio::test]
async fn a_snapshot_is_served_without_the_usage_metering_cannot_read() {
    let f = fixture_over_events().await;
    let (calls, _, sso) = features(&f).await;
    let up = snapshot(&f).await;
    assert!(up.warnings.is_empty(), "{up:?}");
    assert!(level(&up, &calls).used.is_some());

    f.events.unavailable.store(true, Ordering::SeqCst);
    let down = snapshot(&f).await;
    assert_eq!(down.etag, up.etag, "usage is not part of the etag");
    assert_eq!(down.warnings, vec![WARNING.to_owned()]);
    let unread = level(&down, &calls);
    assert_eq!(unread.used, None);
    assert!(unread.usage_unavailable);
    let boolean = level(&down, &sso);
    assert!(!boolean.usage_unavailable);
    assert_eq!(boolean.value, level(&up, &sso).value);
}

/// A limit whose metric is gone, its row pointing nowhere: the feature itself must be fixed.
async fn orphan(f: &Fixture) -> Feature {
    let orphan = feature(
        f,
        "orphan_calls",
        FeatureKind::Limit(LimitCountedBy::Events(METRIC_BANDWIDTH)),
        AtLimit::Deny,
    )
    .await;
    execute(
        f,
        format!(
            "SET session_replication_role = replica;
             UPDATE feature SET metric_id = '{}' WHERE id = '{}';
             SET session_replication_role = origin;",
            BillableMetricId::new().as_uuid(),
            orphan.id.as_uuid()
        ),
    )
    .await;
    orphan
}

#[tokio::test]
async fn a_limit_whose_metric_cannot_be_loaded_is_misconfigured_and_never_retried() {
    let f = fixture_over_events().await;
    let (calls, _, _) = features(&f).await;
    let orphan = orphan(&f).await;

    for down in [false, true] {
        f.events.unavailable.store(down, Ordering::SeqCst);
        let refused = check(&f, &orphan).await;
        assert!(!refused.allowed, "{refused:?}");
        assert_eq!(refused.reason, Some(CheckReason::MetricNotFound));
        assert_eq!(refused.reason_type, Some(ReasonType::Misconfigured));
        assert_eq!(refused.retry_after, None, "retrying does not help");
        assert_eq!(refused.levels[0].used, None);
        assert_eq!(refused.levels[0].denial, Some(CheckReason::MetricNotFound));
        assert!(refused.warnings.is_empty(), "{refused:?}");
    }

    f.events.unavailable.store(false, Ordering::SeqCst);
    let read = snapshot(&f).await;
    assert!(read.warnings.is_empty(), "{read:?}");
    let unread = level(&read, &orphan);
    assert_eq!(unread.used, None);
    assert!(!unread.usage_unavailable);
    assert!(level(&read, &calls).used.is_some());
}

fn metered(entitlements: &[EffectiveEntitlement], feature: &Feature) -> EffectiveEntitlementValue {
    entitlements
        .iter()
        .find(|e| e.feature.id == feature.id)
        .map(|e| e.value.clone())
        .unwrap_or_else(|| panic!("{} in the readout", feature.code))
}

#[tokio::test]
async fn effective_entitlements_degrade_per_entry_when_metering_is_late() {
    let f = fixture_over_events().await;
    let (calls, _, sso) = features(&f).await;
    let effective = || async {
        f.setup
            .services
            .get_effective_entitlements(CUST_UBER_ID, TENANT_ID)
            .await
            .unwrap()
    };
    let up = effective().await;
    assert!(effective_usage_warnings(&up).is_empty());
    match metered(&up, &calls) {
        EffectiveEntitlementValue::Metered { usage, .. } => {
            assert_eq!(usage.consumed, Some(Decimal::ZERO));
            assert!(!usage.unavailable);
        }
        other => panic!("{other:?}"),
    }

    f.events.stalled.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let late = effective().await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(effective_usage_warnings(&late), vec![WARNING.to_owned()]);
    match metered(&late, &calls) {
        EffectiveEntitlementValue::Metered {
            usage,
            limit,
            limit_reached,
            ..
        } => {
            assert_eq!(usage.consumed, None);
            assert_eq!(usage.remaining, None);
            assert!(usage.unavailable);
            assert_eq!(limit, Some(Decimal::from(100)));
            assert!(!limit_reached);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        metered(&late, &sso),
        EffectiveEntitlementValue::Boolean { enabled: true }
    ));

    let limits = f
        .setup
        .services
        .portal_usage_limits(TENANT_ID, CUST_UBER_ID)
        .await
        .unwrap();
    let limit = limits
        .iter()
        .find(|l| l.feature_id == calls.id)
        .expect("the denying limit is listed");
    assert!(limit.usage_unavailable);
    assert!(limit.groups.is_empty() && !limit.reached && !limit.warning);
}

#[tokio::test]
async fn the_subscription_usage_degrades_per_entry_when_metering_is_down() {
    let f = fixture_over_events().await;
    let subscription_id = linear_subscription(&f).await;
    let (calls, _, _) = features(&f).await;
    let bar = |usage: &meteroid_store::domain::customer_summary::SubscriptionUsage| {
        usage
            .usage
            .iter()
            .find(|bar| bar.feature_id == calls.id)
            .cloned()
            .expect("the limit is measured")
    };
    let usage = || async {
        f.setup
            .services
            .get_subscription_usage(TENANT_ID, subscription_id, Utc::now().naive_utc())
            .await
            .unwrap()
    };
    let up = bar(&usage().await);
    assert_eq!(up.consumed, Some(Decimal::ZERO));
    assert!(!up.usage_unavailable);

    f.events.unavailable.store(true, Ordering::SeqCst);
    let down = bar(&usage().await);
    assert_eq!(down.consumed, None);
    assert!(down.usage_unavailable);
    assert_eq!(down.pct, None);
    assert!(!down.refused);
    assert_eq!(down.limit, Some(Decimal::from(100)));
}

#[tokio::test]
async fn the_batched_snapshots_carry_the_warning() {
    let f = fixture_over_events().await;
    let (calls, _, _) = features(&f).await;
    let store = &f.setup.store;
    store
        .create_resource_type(ResourceTypeNew {
            tenant_id: TENANT_ID,
            code: "site".into(),
            name: "Site".into(),
            plural_name: "Sites".into(),
            parent_type_id: None,
            keys: vec!["site_id".into()],
            attributes: BTreeMap::new(),
        })
        .await
        .unwrap();
    let site = store
        .upsert_resource(ResourceUpsert {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            resource_type: AliasOr::Alias("site".into()),
            key_value: "s1".into(),
            name: None,
            attributes: BTreeMap::new(),
            archived: false,
        })
        .await
        .unwrap()
        .id;
    let snapshots = || async {
        store
            .entitlement_snapshots(
                TENANT_ID,
                CUST_UBER_ID,
                &[site],
                Some(std::slice::from_ref(&calls.id)),
            )
            .await
            .unwrap()
    };
    let up = snapshots().await;
    assert!(up[0].warnings.is_empty(), "{:?}", up[0].warnings);

    f.events.unavailable.store(true, Ordering::SeqCst);
    let down = snapshots().await;
    assert_eq!(down[0].resource_id, site);
    assert_eq!(down[0].warnings, vec![WARNING.to_owned()]);
    let level = &down[0].features[0].levels[0];
    assert_eq!(level.used, None);
    assert!(level.usage_unavailable);

    let read = store
        .entitlement_snapshot_features(
            TENANT_ID,
            CUST_UBER_ID,
            &ResourceInput::default(),
            std::slice::from_ref(&calls.id),
        )
        .await
        .unwrap();
    assert_eq!(
        meteroid_store::domain::entitlement_checks::usage_warnings(&read),
        vec![WARNING.to_owned()]
    );
}
