//! Notices published once each: an override's end (R-148), a trial's end (R-158).

use common_domain::actor::Actor;
use common_domain::ids::EntitlementEntityId;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use meteroid_store::domain::entitlement_fold::GrantValue;
use meteroid_store::domain::entitlements::{
    EntitlementNew, EntitlementSpec, EntitlementUpdate, FeatureKind, FeatureNew,
};
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::services::override_notices::notify_override_endings;
use meteroid_store::services::trial_notices::notify_trial_endings;
use rstest::rstest;

use super::phases::{at, day};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

async fn feature(env: &TestEnv, code: &str) -> common_domain::ids::FeatureId {
    env.store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: code.to_string(),
                code: code.to_string(),
                description: None,
                kind: FeatureKind::Boolean,
                unit: None,
                level_type_id: None,
                policy: None,
                default: None,
            },
        )
        .await
        .expect("a feature")
        .id
}

async fn override_until(
    env: &TestEnv,
    feature: common_domain::ids::FeatureId,
    until: chrono::DateTime<chrono::Utc>,
) -> common_domain::ids::EntitlementId {
    let mut spec = EntitlementSpec::new(feature, GrantValue::Boolean(true));
    spec.valid_until = Some(until);
    spec.reason = Some("pilot".to_string());
    env.store()
        .create_entitlement(
            &Actor::System,
            EntitlementNew {
                tenant_id: TENANT_ID,
                entity: EntitlementEntityId::Customer(CUST_UBER_ID),
                resource_id: None,
                spec,
            },
        )
        .await
        .expect("an override with an end")
        .id
}

/// An override to March 15 is noticed once as ending soon in its last week, then once as ended;
/// extended to March 31, both are noticed again for the new end. A revoked one is never noticed.
/// Rules: R-148
#[rstest]
#[tokio::test]
async fn an_override_end_is_noticed_once_each_way_and_again_when_moved(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(3, 1, 12));
    let pilot = override_until(&env, feature(&env, "sso").await, at(3, 15, 0)).await;
    let revoked = override_until(&env, feature(&env, "audit_log").await, at(3, 20, 0)).await;
    env.store()
        .delete_entitlement(&Actor::System, revoked, TENANT_ID)
        .await
        .expect("revoked");
    let store = env.store();

    assert_eq!(
        notify_override_endings(store, at(3, 1, 12)).await.unwrap(),
        (0, 0)
    );
    assert_eq!(
        notify_override_endings(store, at(3, 9, 12)).await.unwrap(),
        (1, 0)
    );
    assert_eq!(
        notify_override_endings(store, at(3, 9, 13)).await.unwrap(),
        (0, 0)
    );

    env.store()
        .update_entitlement(
            &Actor::System,
            pilot,
            TENANT_ID,
            EntitlementUpdate {
                valid_until: Some(Some(at(3, 31, 0))),
                ..EntitlementUpdate::default()
            },
        )
        .await
        .expect("extended to March 31");
    assert_eq!(
        notify_override_endings(store, at(3, 15, 12)).await.unwrap(),
        (0, 0),
        "neither the old end nor the revoked override is noticed"
    );
    assert_eq!(
        notify_override_endings(store, at(3, 25, 12)).await.unwrap(),
        (1, 0)
    );
    assert_eq!(
        notify_override_endings(store, at(4, 1, 12)).await.unwrap(),
        (0, 1)
    );
    assert_eq!(
        notify_override_endings(store, at(4, 2, 12)).await.unwrap(),
        (0, 0)
    );
}

/// A two-week trial from January 1 is noticed once seven days before it ends, January 15; moved
/// to January 22, it is noticed again for that end. A trial of five days is noticed at its start.
/// Rules: R-158
#[rstest]
#[tokio::test]
async fn a_trial_end_is_noticed_once_and_again_when_moved(#[future] test_env: TestEnv) {
    use diesel_models::schema::subscription_trial::dsl as t;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_days(14)
        .create(env.services())
        .await;
    let store = env.store();

    assert_eq!(notify_trial_endings(store, at(1, 7, 12)).await.unwrap(), 0);
    assert_eq!(notify_trial_endings(store, at(1, 8, 12)).await.unwrap(), 1);
    assert_eq!(notify_trial_endings(store, at(1, 8, 13)).await.unwrap(), 0);

    let mut conn = env.conn().await;
    diesel::update(t::subscription_trial.filter(t::subscription_id.eq(sub_id)))
        .set(t::until_date.eq(day(1, 22)))
        .execute(&mut conn)
        .await
        .expect("the trial moved to January 22");
    assert_eq!(notify_trial_endings(store, at(1, 14, 12)).await.unwrap(), 0);
    assert_eq!(notify_trial_endings(store, at(1, 15, 12)).await.unwrap(), 1);

    env.set_now(at(1, 15, 12));
    subscription()
        .customer(CUST_SPOTIFY_ID)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 15))
        .on_start()
        .trial_days(5)
        .create(env.services())
        .await;
    assert_eq!(notify_trial_endings(store, at(1, 15, 13)).await.unwrap(), 1);
}
