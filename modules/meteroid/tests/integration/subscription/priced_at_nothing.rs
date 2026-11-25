//! A standard plan whose every price is zero is free: subscribing to it needs no payment method,
//! and it can be the plan a new customer signs up to.
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::repositories::ProductFamilyJourneysInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{SubscriptionAssertExt, TestEnv, subscription, test_env};

/// A standard plan of a platform fee at `fee` a month and an included product.
async fn standard_plan(env: &TestEnv, name: &'static str, fee: Decimal) -> (PlanId, PlanVersionId) {
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(plan, name, version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                fee,
            ),
            SeedComp::included(PriceComponentId::new(), "Seats", PRODUCT_SEATS_ID),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("a standard plan");
    (plan, version)
}

fn journey(plan: PlanId) -> ProductFamilyJourney {
    ProductFamilyJourney {
        product_family_id: PRODUCT_FAMILY_ID,
        signup_plan_id: Some(plan),
        signup_option: None,
        period_switch: true,
        self_serve_downgrade: false,
    }
}

/// Charged automatically with no payment provider: refused on a priced plan, accepted on one
/// priced at nothing, which has nothing to collect.
#[tokio::test]
async fn a_standard_plan_priced_at_nothing_needs_no_payment_method() {
    let env = test_env().await;
    let (_, hobby) = standard_plan(&env, "Hobby", Decimal::ZERO).await;
    let (_, team) = standard_plan(&env, "Team", dec!(20)).await;
    let subscribe = |version| {
        env.services().insert_subscription(
            Actor::User { id: USER_ID },
            subscription()
                .plan_version(version)
                .on_start()
                .no_trial()
                .auto_charge()
                .request(),
            TENANT_ID,
        )
    };

    let created = subscribe(hobby)
        .await
        .expect("nothing to charge: no payment method needed");
    env.get_subscription(created.id).await.assert().is_active();
    let refused = subscribe(team)
        .await
        .expect_err("a priced plan charged automatically needs a payment provider");
    assert!(
        format!("{refused:?}").contains("Automatic charging"),
        "{refused:?}"
    );
}

/// A new customer has no payment method: a standard plan priced at nothing can sign them up, a
/// priced one without a free trial cannot.
#[tokio::test]
async fn a_standard_plan_priced_at_nothing_can_be_the_signup_plan() {
    let env = test_env().await;
    let (hobby, _) = standard_plan(&env, "Hobby", Decimal::ZERO).await;
    let (team, _) = standard_plan(&env, "Team", dec!(20)).await;
    let save = |plan| {
        env.store()
            .save_product_family_journey(TENANT_ID, journey(plan))
    };

    assert!(
        save(team).await.is_err(),
        "a priced plan with no trial cannot sign up a customer"
    );
    let saved = save(hobby).await.expect("a plan priced at nothing");
    assert_eq!(saved.signup_plan_id, Some(hobby));

    let offer = env
        .services()
        .signup_offer(TENANT_ID, None, "EUR")
        .await
        .expect("the sign-up offer");
    let created = env
        .services()
        .subscribe_to_signup_plan(Actor::System, TENANT_ID, CUST_UBER_ID, offer)
        .await
        .expect("subscribed with no payment method");
    env.get_subscription(created.id).await.assert().is_active();
}
