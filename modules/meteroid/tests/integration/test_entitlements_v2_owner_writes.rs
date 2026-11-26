//! The owner writes API v2 reaches beyond the catalog lists (D4): a subscription's overrides
//! replaced as a list under its customer's inputs, a discarded draft leaving no row behind, and a
//! feature created with its quantity and defaults, all or nothing.
use crate::data::ids::{
    CUST_UBER_ID, PLAN_LEETCODE_ID, PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_2_LEETCODE_ID,
    TENANT_ID,
};
use crate::harness::{TestEnv, test_env};
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, EntitlementEntityId, FeatureId, ManagedQuantityId, SubscriptionId,
};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::query::credit_inputs;
use meteroid_store::StoreResult;
use meteroid_store::domain::entitlement_fold::{Effect, GrantValue, LimitValue};
use meteroid_store::domain::entitlements::{
    Entitlement, EntitlementSpec, Feature, FeatureKind, FeatureNew, LimitCountedBy,
};
use meteroid_store::domain::managed_quantities::{
    ManagedQuantityNew, QuantityDecrease, QuantityIncrease,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::{ManagedQuantitiesInterface, PlansInterface};
use rstest::rstest;
use rust_decimal::Decimal;

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

fn new_feature(code: &str, kind: FeatureKind) -> FeatureNew {
    FeatureNew {
        tenant_id: TENANT_ID,
        product_id: None,
        name: code.into(),
        code: code.into(),
        description: None,
        kind,
        unit: None,
        level_type_id: None,
        policy: None,
        default: None,
    }
}

async fn feature(env: &TestEnv, code: &str, kind: FeatureKind) -> Feature {
    env.store()
        .create_feature(Actor::System, new_feature(code, kind))
        .await
        .expect("a feature")
}

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

/// Every row the owner holds, revoked ones included.
async fn rows_of_owner(env: &TestEnv, owner: EntitlementEntityId) -> i64 {
    use diesel_models::schema::entitlement::dsl as e;
    e::entitlement
        .filter(e::tenant_id.eq(TENANT_ID))
        .filter(e::entity_id.eq(owner.as_uuid()))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("a count of the owner's rows")
}

/// An active subscription of Uber, written directly: the writes only read its customer and
/// status.
async fn subscribe(env: &TestEnv) -> SubscriptionId {
    use diesel::sql_types::{Date, Uuid};
    let id = SubscriptionId::new();
    diesel::sql_query(
        "INSERT INTO subscription (id, customer_id, billing_day_anchor, tenant_id, start_date,
             plan_version_id, created_at, net_terms, currency, mrr_cents, period,
             pending_checkout, activation_condition, status, current_period_start,
             auto_advance_invoices, charge_automatically, backdate_invoices, custom_properties,
             tax_inclusive, lifecycle_suspended, resource_id, activated_at)
         VALUES ($1, $2, 1, $3, $4, $5, now(), 0, 'EUR', 0, 'MONTHLY', false, 'ON_START',
             'ACTIVE', $4, true, false, false, '{}', false, false, NULL, now())",
    )
    .bind::<Uuid, _>(id.as_uuid())
    .bind::<Uuid, _>(CUST_UBER_ID.as_uuid())
    .bind::<Uuid, _>(TENANT_ID.as_uuid())
    .bind::<Date, _>(Utc::now().date_naive())
    .bind::<Uuid, _>(PLAN_VERSION_1_LEETCODE_ID.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect("a subscription");
    id
}

async fn end(env: &TestEnv, id: SubscriptionId) {
    use diesel::sql_types::Uuid;
    diesel::sql_query("UPDATE subscription SET status = 'CANCELLED' WHERE id = $1")
        .bind::<Uuid, _>(id.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("ended");
}

async fn inputs_token(env: &TestEnv) -> Option<i64> {
    credit_inputs::customer_token(&mut env.conn().await, TENANT_ID, CUST_UBER_ID)
        .await
        .expect("the customer's inputs token")
}

/// A subscription's overrides written whole, as the other owner lists are: a repeated row is kept,
/// a changed one succeeded in its lineage, one left out revoked (never deleted), under the
/// customer's lock, whose inputs move. Only `set` rows; once the subscription has ended its rows
/// no longer change.
#[rstest]
#[tokio::test]
async fn a_subscriptions_overrides_are_replaced_as_a_list(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let sso = feature(&env, "sso", FeatureKind::Boolean).await;
    let seats = feature(&env, "seats", FeatureKind::Limit(LimitCountedBy::App)).await;
    let reports = feature(&env, "reports", FeatureKind::Boolean).await;
    let subscription_id = subscribe(&env).await;
    let owner = EntitlementEntityId::Subscription(subscription_id);
    let seats_at = |n: i64| EntitlementSpec::new(seats.id, limit(n));

    let before = inputs_token(&env).await;
    let first = store
        .replace_entitlements(&actor, TENANT_ID, owner, vec![on(&sso), seats_at(10)])
        .await
        .expect("the first list");
    assert_eq!(first.len(), 2);
    assert!(
        first.iter().all(|row| row.effect == Effect::Set),
        "the subscription tier's default: {first:?}"
    );
    let bumped = inputs_token(&env).await;
    assert!(
        bumped.is_some() && bumped != before,
        "the customer's inputs move: {before:?} then {bumped:?}"
    );
    let (sso_row, seats_row) = (row_of(&first, &sso), row_of(&first, &seats));

    let second = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            owner,
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
    assert_eq!(history(&env, &seats_row).await.len(), 2);

    let third = store
        .replace_entitlements(&actor, TENANT_ID, owner, vec![seats_at(20), on(&reports)])
        .await
        .expect("the third list");
    assert_eq!(third.len(), 2, "{third:?}");
    let sso_history = history(&env, &sso_row).await;
    assert_eq!(sso_history.len(), 1, "revoked, never deleted");
    assert!(sso_history[0].revoked_at.is_some());

    let raising = EntitlementSpec {
        effect: Some(Effect::RaiseTo),
        ..on(&sso)
    };
    let refused = invalid(
        store
            .replace_entitlements(&actor, TENANT_ID, owner, vec![raising])
            .await,
    );
    assert!(refused.contains("not allowed"), "{refused}");

    end(&env, subscription_id).await;
    let ended = store
        .replace_entitlements(&actor, TENANT_ID, owner, vec![])
        .await
        .expect_err("an ended subscription");
    assert!(
        matches!(ended.current_context(), StoreError::Conflict(m) if m.contains("ended")),
        "{ended:?}"
    );
    let kept = store
        .list_entitlements_by_entity(owner, TENANT_ID)
        .await
        .expect("rows");
    assert_eq!(kept.len(), 2, "nothing written: {kept:?}");
}

/// A discarded draft goes with every row it held, revoked ones included; the plan, which keeps
/// its published version, keeps its own rows.
#[rstest]
#[tokio::test]
async fn a_discarded_draft_leaves_no_row_behind(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let sso = feature(&env, "sso", FeatureKind::Boolean).await;
    let seats = feature(&env, "seats", FeatureKind::Limit(LimitCountedBy::App)).await;
    let draft = EntitlementEntityId::PlanVersion(PLAN_VERSION_2_LEETCODE_ID);
    let plan = EntitlementEntityId::Plan(PLAN_LEETCODE_ID);

    let rows = store
        .replace_entitlements(
            &actor,
            TENANT_ID,
            draft,
            vec![on(&sso), EntitlementSpec::new(seats.id, limit(10))],
        )
        .await
        .expect("the draft's rows");
    store
        .delete_entitlement(&actor, row_of(&rows, &seats).id, TENANT_ID)
        .await
        .expect("a row of the draft revoked");
    store
        .replace_entitlements(&actor, TENANT_ID, plan, vec![on(&sso)])
        .await
        .expect("the plan's rows");
    assert_eq!(
        rows_of_owner(&env, draft).await,
        2,
        "an open row and a revoked one"
    );

    store
        .discard_draft_plan_version(Actor::System, PLAN_VERSION_2_LEETCODE_ID, TENANT_ID)
        .await
        .expect("the draft discarded");
    assert_eq!(rows_of_owner(&env, draft).await, 0, "no orphan row");
    for row in &rows {
        assert!(history(&env, row).await.is_empty(), "{row:?}");
    }
    let plan_rows = store
        .list_entitlements_by_entity(plan, TENANT_ID)
        .await
        .expect("the plan's rows");
    assert_eq!(plan_rows.len(), 1, "{plan_rows:?}");
}

async fn exists(env: &TestEnv, code: &str) -> bool {
    env.store()
        .get_feature_by_id_or_code(AliasOr::Alias(code.to_owned()), TENANT_ID)
        .await
        .is_ok()
}

async fn per_unit_of(env: &TestEnv, quantity: ManagedQuantityId) -> Vec<FeatureId> {
    env.store()
        .list_managed_quantities(TENANT_ID)
        .await
        .expect("quantities")
        .into_iter()
        .find(|q| q.id == quantity)
        .expect("the quantity")
        .per_unit_features
}

/// A feature, the quantity its limit counts per unit of and its defaults are written in one
/// transaction: a refusal of any part leaves no feature, no quantity link and no row.
#[rstest]
#[tokio::test]
async fn a_feature_is_created_with_its_quantity_and_defaults_or_not_at_all(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let store = env.store();
    let actor = Actor::System;
    let other = feature(&env, "other", FeatureKind::Boolean).await;
    let users = store
        .create_managed_quantity(ManagedQuantityNew {
            tenant_id: TENANT_ID,
            code: "users".into(),
            name: "Users".into(),
            on_increase: QuantityIncrease::default(),
            on_decrease: QuantityDecrease::default(),
        })
        .await
        .expect("a quantity");
    let projects = || new_feature("projects", FeatureKind::Limit(LimitCountedBy::App));

    let id = FeatureId::new();
    let foreign = invalid(
        store
            .create_feature_with_defaults(&actor, id, projects(), Some(users.id), vec![on(&other)])
            .await,
    );
    assert!(foreign.contains("for that feature"), "{foreign}");
    let negative = invalid(
        store
            .create_feature_with_defaults(
                &actor,
                id,
                projects(),
                Some(users.id),
                vec![EntitlementSpec::new(id, limit(-1))],
            )
            .await,
    );
    assert!(negative.contains("negative"), "{negative}");
    let not_a_limit = invalid(
        store
            .create_feature_with_defaults(
                &actor,
                FeatureId::new(),
                new_feature("flag", FeatureKind::Boolean),
                Some(users.id),
                vec![],
            )
            .await,
    );
    assert!(not_a_limit.contains("only a limit"), "{not_a_limit}");
    assert!(
        store.get_feature(id, TENANT_ID).await.is_err(),
        "no feature"
    );
    assert!(!exists(&env, "projects").await);
    assert!(!exists(&env, "flag").await);
    assert!(
        per_unit_of(&env, users.id).await.is_empty(),
        "the quantity counts no feature"
    );

    let created = store
        .create_feature_with_defaults(
            &actor,
            id,
            projects(),
            Some(users.id),
            vec![EntitlementSpec::new(id, limit(5))],
        )
        .await
        .expect("written together");
    assert_eq!(created.id, id, "under the id asked");
    let [default] = created.defaults.as_slice() else {
        panic!("one default: {:?}", created.defaults);
    };
    assert_eq!(default.value, limit(5));
    assert_eq!(default.effect, Effect::Set);
    assert!(
        matches!(default.entity, EntitlementEntityId::Feature(owner) if owner == id),
        "{default:?}"
    );
    assert_eq!(per_unit_of(&env, users.id).await, vec![id]);

    let taken = store
        .create_feature_with_defaults(&actor, FeatureId::new(), projects(), None, vec![])
        .await
        .expect_err("the code is taken");
    assert!(
        matches!(taken.current_context(), StoreError::DuplicateValue { .. }),
        "{taken:?}"
    );
    assert_eq!(per_unit_of(&env, users.id).await, vec![id]);
}
