//! Owner lists written whole (D4): `replace_entitlements` keeps what it repeats, gives a changed
//! row a successor, revokes what it leaves out and deletes a draft's; a product's and a feature's
//! defaults the same way. Features read in keyset pages.
use crate::data::ids::{
    CUST_UBER_ID, PLAN_LEETCODE_ID, PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_2_LEETCODE_ID,
    PRODUCT_PLATFORM_FEE_ID, PRODUCT_SEATS_ID, TENANT_ID,
};
use crate::harness::{TestEnv, test_env};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, EntitlementEntityId, FeatureId, ProductId};
use meteroid_store::StoreResult;
use meteroid_store::domain::FeatureListOrder;
use meteroid_store::domain::entitlement_fold::{GrantValue, LimitValue};
use meteroid_store::domain::entitlements::{
    Entitlement, EntitlementSpec, Feature, FeatureKind, FeatureNew, FeatureUpdate, LimitCountedBy,
};
use meteroid_store::domain::enums::FeatureStatusEnum;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::entitlements::{EntitlementsInterface, FeaturePageFilter};
use meteroid_store::repositories::resources::ResourceTypeNew;
use rstest::rstest;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

fn limit(n: i64) -> GrantValue {
    GrantValue::Limit(LimitValue::Amount(Decimal::from(n)))
}

fn on(feature: &Feature) -> EntitlementSpec {
    EntitlementSpec::new(feature.id, GrantValue::Boolean(true))
}

fn invalid<T: std::fmt::Debug>(result: StoreResult<T>) -> String {
    match result.expect_err("refused").current_context() {
        StoreError::InvalidArgument(message) => message.clone(),
        other => panic!("expected a 400, got {other:?}"),
    }
}

fn new_feature(
    name: &str,
    kind: FeatureKind,
    product_id: Option<ProductId>,
    default: Option<GrantValue>,
) -> FeatureNew {
    FeatureNew {
        tenant_id: TENANT_ID,
        product_id,
        name: name.into(),
        code: name.into(),
        description: None,
        kind,
        unit: None,
        level_type_id: None,
        policy: None,
        default,
    }
}

async fn feature(env: &TestEnv, new: FeatureNew) -> Feature {
    env.store()
        .create_feature(Actor::System, new)
        .await
        .expect("a feature")
}

/// The open row of `feature` among `rows`.
fn row_of(rows: &[Entitlement], feature: &Feature) -> Entitlement {
    rows.iter()
        .find(|row| row.feature_id == feature.id)
        .unwrap_or_else(|| panic!("a row of {}: {rows:?}", feature.code))
        .clone()
}

async fn history(env: &TestEnv, row: &Entitlement) -> Vec<Entitlement> {
    env.store()
        .get_entitlement_history(row.id, TENANT_ID)
        .await
        .expect("the lineage's history")
}

/// A plan's list replaced whole: a repeated row is kept as it is, a changed one gets a successor
/// in its lineage, one left out is revoked with its history kept, a new one is inserted. Two rows
/// of one key over the same days, a row of an archived feature, a published version and a
/// customer are refused, and an archived feature's rows are left as they are.
#[rstest]
#[tokio::test]
async fn a_replaced_list_keeps_changes_and_revokes_its_rows(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let sso = feature(&env, new_feature("sso", FeatureKind::Boolean, None, None)).await;
    let seats = feature(
        &env,
        new_feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None),
    )
    .await;
    let reports = feature(
        &env,
        new_feature("reports", FeatureKind::Boolean, None, None),
    )
    .await;
    let plan = EntitlementEntityId::Plan(PLAN_LEETCODE_ID);
    let seats_at = |n| EntitlementSpec::new(seats.id, limit(n));

    let first = store
        .replace_entitlements(&actor, TENANT_ID, plan, vec![on(&sso), seats_at(10)])
        .await
        .expect("the first list");
    assert_eq!(first.len(), 2);
    let (sso_row, seats_row) = (row_of(&first, &sso), row_of(&first, &seats));

    let second = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            plan,
            vec![on(&sso), seats_at(20), on(&reports)],
        )
        .await
        .expect("the second list");
    assert_eq!(second.len(), 3);
    assert_eq!(
        row_of(&second, &sso).version_id,
        sso_row.version_id,
        "repeated, kept as it is"
    );
    let changed = row_of(&second, &seats);
    assert_eq!(changed.id, seats_row.id, "changed, in its lineage");
    assert_ne!(changed.version_id, seats_row.version_id);
    assert_eq!(changed.value, limit(20));
    let seats_history = history(&env, &seats_row).await;
    assert_eq!(seats_history.len(), 2);
    assert!(seats_history[0].revoked_at.is_some());
    let reports_row = row_of(&second, &reports);

    let overlapping = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            plan,
            vec![on(&sso), seats_at(20), seats_at(30), on(&reports)],
        )
        .await
        .expect_err("two rows of one key over the same days");
    assert!(
        matches!(
            overlapping.current_context(),
            StoreError::DuplicateValue { .. }
        ),
        "{overlapping:?}"
    );
    let unchanged = store
        .list_entitlements_by_entity(plan, TENANT_ID)
        .await
        .expect("rows");
    assert_eq!(unchanged.len(), 3, "nothing written: {unchanged:?}");

    store
        .set_feature_status(
            Actor::System,
            reports.id,
            TENANT_ID,
            FeatureStatusEnum::Archived,
        )
        .await
        .expect("archived");
    let refused = invalid(
        store
            .replace_entitlements(&actor, TENANT_ID, plan, vec![seats_at(20), on(&reports)])
            .await,
    );
    assert!(refused.contains("archived"), "{refused}");
    let third = store
        .replace_entitlements(&actor, TENANT_ID, plan, vec![seats_at(20)])
        .await
        .expect("the third list");
    assert_eq!(third.len(), 1, "{third:?}");
    let sso_history = history(&env, &sso_row).await;
    assert_eq!(sso_history.len(), 1, "revoked, not deleted");
    assert!(sso_history[0].revoked_at.is_some());
    store
        .set_feature_status(
            Actor::System,
            reports.id,
            TENANT_ID,
            FeatureStatusEnum::Active,
        )
        .await
        .expect("restored");
    let restored = store
        .list_entitlements_by_entity(plan, TENANT_ID)
        .await
        .expect("rows");
    assert_eq!(
        row_of(&restored, &reports).version_id,
        reports_row.version_id,
        "an archived feature's row is left as it is"
    );

    let published = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            EntitlementEntityId::PlanVersion(PLAN_VERSION_1_LEETCODE_ID),
            vec![on(&sso)],
        )
        .await
        .expect_err("a published version");
    assert!(
        matches!(published.current_context(), StoreError::FailedPrecondition(m) if m.contains("through a draft")),
        "{published:?}"
    );
    invalid(
        store
            .replace_entitlements(
                &actor,
                TENANT_ID,
                EntitlementEntityId::Customer(CUST_UBER_ID),
                vec![on(&sso)],
            )
            .await,
    );
}

/// A draft version was never published: its replaced rows are deleted, a changed one keeping its
/// lineage id.
#[rstest]
#[tokio::test]
async fn a_drafts_replaced_rows_are_deleted(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let sso = feature(&env, new_feature("sso", FeatureKind::Boolean, None, None)).await;
    let seats = feature(
        &env,
        new_feature("seats", FeatureKind::Limit(LimitCountedBy::App), None, None),
    )
    .await;
    let draft = EntitlementEntityId::PlanVersion(PLAN_VERSION_2_LEETCODE_ID);

    let first = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            draft,
            vec![on(&sso), EntitlementSpec::new(seats.id, limit(10))],
        )
        .await
        .expect("the draft's first list");
    let (sso_row, seats_row) = (row_of(&first, &sso), row_of(&first, &seats));
    let second = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            draft,
            vec![EntitlementSpec::new(seats.id, limit(20))],
        )
        .await
        .expect("the draft's second list");
    assert_eq!(second.len(), 1);
    assert!(
        history(&env, &sso_row).await.is_empty(),
        "left out of a draft, deleted"
    );
    let changed = row_of(&second, &seats);
    assert_eq!(changed.id, seats_row.id, "the lineage id is kept");
    assert_eq!(changed.value, limit(20));
    assert_eq!(
        history(&env, &seats_row).await.len(),
        1,
        "no history on a draft"
    );
}

/// A product's defaults name only its own live features: another product's feature, or a global
/// one, is refused and nothing changes. A feature left out loses its defaults.
#[rstest]
#[tokio::test]
async fn a_products_defaults_are_its_own_features(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let export = feature(
        &env,
        new_feature(
            "export",
            FeatureKind::Boolean,
            Some(PRODUCT_PLATFORM_FEE_ID),
            None,
        ),
    )
    .await;
    let seat_reports = feature(
        &env,
        new_feature(
            "seat_reports",
            FeatureKind::Boolean,
            Some(PRODUCT_SEATS_ID),
            None,
        ),
    )
    .await;
    let global = feature(
        &env,
        new_feature("global", FeatureKind::Boolean, None, None),
    )
    .await;
    let defaults_of = |feature: FeatureId| async move {
        store
            .get_feature(feature, TENANT_ID)
            .await
            .expect("the feature")
            .defaults
    };

    for stranger in [&seat_reports, &global] {
        let refused = invalid(
            store
                .set_product_feature_defaults(
                    &actor,
                    TENANT_ID,
                    PRODUCT_PLATFORM_FEE_ID,
                    vec![on(&export), on(stranger)],
                )
                .await,
        );
        assert!(refused.contains("not a live feature"), "{refused}");
    }
    assert!(defaults_of(export.id).await.is_empty(), "nothing written");

    store
        .set_product_feature_defaults(
            &actor,
            TENANT_ID,
            PRODUCT_PLATFORM_FEE_ID,
            vec![on(&export)],
        )
        .await
        .expect("the product's defaults");
    let set = defaults_of(export.id).await;
    assert_eq!(set.len(), 1);
    assert!(defaults_of(seat_reports.id).await.is_empty());

    store
        .set_product_feature_defaults(&actor, TENANT_ID, PRODUCT_PLATFORM_FEE_ID, vec![])
        .await
        .expect("no defaults");
    assert!(defaults_of(export.id).await.is_empty());
    assert!(
        history(&env, &set[0]).await[0].revoked_at.is_some(),
        "revoked, its history kept"
    );
}

/// A feature's edit and its defaults are written together: a default refused writes neither the
/// edit nor the defaults; both accepted, the changed default keeps its lineage.
#[rstest]
#[tokio::test]
async fn a_feature_and_its_defaults_are_written_together(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let sso = feature(
        &env,
        new_feature(
            "sso",
            FeatureKind::Boolean,
            None,
            Some(GrantValue::Boolean(true)),
        ),
    )
    .await;
    let other = feature(&env, new_feature("other", FeatureKind::Boolean, None, None)).await;
    let renamed = |name: &str| FeatureUpdate {
        name: Some(name.to_owned()),
        ..FeatureUpdate::default()
    };

    let refused = invalid(
        store
            .update_feature_with_defaults(
                &actor,
                sso.id,
                TENANT_ID,
                Some(renamed("Single sign-on")),
                None,
                Some(vec![on(&other)]),
            )
            .await,
    );
    assert!(refused.contains("for that feature"), "{refused}");
    let kept = store.get_feature(sso.id, TENANT_ID).await.expect("sso");
    assert_eq!(kept.name, "sso", "the edit is not written either");
    assert_eq!(kept.defaults, sso.defaults);

    let updated = store
        .update_feature_with_defaults(
            &actor,
            sso.id,
            TENANT_ID,
            Some(renamed("Single sign-on")),
            None,
            Some(vec![EntitlementSpec::new(
                sso.id,
                GrantValue::Boolean(false),
            )]),
        )
        .await
        .expect("both written");
    assert_eq!(updated.name, "Single sign-on");
    let [default] = updated.defaults.as_slice() else {
        panic!("one default: {:?}", updated.defaults);
    };
    assert_eq!(default.value, GrantValue::Boolean(false));
    assert_eq!(default.id, sso.defaults[0].id, "the default's lineage");
}

/// Every page of `filter`, two features at a time, by name.
async fn paged(env: &TestEnv, filter: FeaturePageFilter) -> Vec<String> {
    let mut names = Vec::new();
    let mut after = None;
    loop {
        let page = env
            .store()
            .list_features_after(TENANT_ID, filter.clone(), after, 2)
            .await
            .expect("a page");
        let Some(last) = page.last() else {
            return names;
        };
        after = Some(last.id);
        names.extend(page.into_iter().map(|f| f.name));
    }
}

/// Keyset pages of features, by id, by name or by product (by product name, global features
/// last), each feature with its defaults; narrowed to a product or a level. A cursor that
/// is not a feature of the tenant is refused.
#[rstest]
#[tokio::test]
async fn features_are_read_in_keyset_pages(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let site = store
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
        .expect("a resource type");
    let boolean = |name: &str, product| new_feature(name, FeatureKind::Boolean, product, None);
    let mut created = [
        feature(
            &env,
            new_feature(
                "alpha",
                FeatureKind::Boolean,
                None,
                Some(GrantValue::Boolean(true)),
            ),
        )
        .await,
        feature(&env, boolean("zeta", None)).await,
        feature(
            &env,
            FeatureNew {
                level_type_id: Some(site.id),
                ..boolean("pages", None)
            },
        )
        .await,
        feature(&env, boolean("gamma", Some(PRODUCT_PLATFORM_FEE_ID))).await,
        feature(&env, boolean("beta", Some(PRODUCT_PLATFORM_FEE_ID))).await,
        feature(&env, boolean("mid", Some(PRODUCT_SEATS_ID))).await,
    ];

    created.sort_by_key(|f| f.id.as_uuid());
    let by_id: Vec<String> = created.iter().map(|f| f.name.clone()).collect();
    assert_eq!(paged(&env, FeaturePageFilter::default()).await, by_id);
    let order = |order| FeaturePageFilter {
        order: Some(order),
        ..FeaturePageFilter::default()
    };
    assert_eq!(
        paged(&env, order(FeatureListOrder::Name)).await,
        vec!["alpha", "beta", "gamma", "mid", "pages", "zeta"]
    );
    assert_eq!(
        paged(&env, order(FeatureListOrder::Product)).await,
        vec!["beta", "gamma", "mid", "alpha", "pages", "zeta"],
        "Platform Fee, then Seats, then global"
    );
    assert_eq!(
        paged(
            &env,
            FeaturePageFilter {
                product_id: Some(PRODUCT_PLATFORM_FEE_ID),
                ..order(FeatureListOrder::Name)
            }
        )
        .await,
        vec!["beta", "gamma"]
    );
    assert_eq!(
        paged(
            &env,
            FeaturePageFilter {
                level: Some(None),
                ..order(FeatureListOrder::Name)
            }
        )
        .await,
        vec!["alpha", "beta", "gamma", "mid", "zeta"],
        "per customer"
    );
    assert_eq!(
        paged(
            &env,
            FeaturePageFilter {
                level: Some(Some(site.id)),
                ..FeaturePageFilter::default()
            }
        )
        .await,
        vec!["pages"]
    );

    let first = store
        .list_features_after(TENANT_ID, order(FeatureListOrder::Name), None, 1)
        .await
        .expect("a page");
    assert_eq!(first[0].name, "alpha");
    assert_eq!(first[0].defaults.len(), 1, "with its defaults");

    let refused = invalid(
        store
            .list_features_after(
                TENANT_ID,
                FeaturePageFilter::default(),
                Some(FeatureId::new()),
                2,
            )
            .await,
    );
    assert!(refused.contains("page after"), "{refused}");
}
