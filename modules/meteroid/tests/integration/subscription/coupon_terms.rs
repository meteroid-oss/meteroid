//! Coupon terms: a coupon's validity start, its fixed amounts per currency and who may redeem
//! it; a plan's eligibility, set live.
use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};
use common_domain::actor::Actor;
use common_domain::ids::CouponId;
use meteroid_store::domain::PlanVersionFilter;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew, CouponTerms};
use meteroid_store::domain::eligibility::{Eligibility, PropertyMatch};
use meteroid_store::repositories::coupons::{CouponInterface, CouponInterfaceAuto};
use meteroid_store::repositories::{EligibilityInterface, PlansInterface};

fn fixed(code: &str, amount: i64, currency: &str) -> CouponNew {
    CouponNew {
        code: code.to_string(),
        description: format!("{amount} {currency} off"),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Fixed {
            amount: Decimal::from(amount),
            currency: currency.to_string(),
        },
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: true,
        plan_ids: vec![],
    }
}

async fn create(env: &TestEnv, coupon: CouponNew, terms: CouponTerms) -> CouponId {
    env.store()
        .create_coupon_with_terms(Actor::System, coupon, terms)
        .await
        .expect("coupon created")
        .id
}

/// The refusal of a subscription to the EUR plan with `coupon_id`, `None` when it was created.
async fn refusal(env: &TestEnv, coupon_id: CouponId) -> Option<String> {
    let request = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .request();
    env.services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .err()
        .map(|e| format!("{e:?}"))
}

#[rstest]
#[tokio::test]
async fn a_coupon_is_refused_before_it_is_valid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let later = common_clock::now().naive_utc() + chrono::Duration::days(30);
    let coupon_id = create(
        &env,
        fixed("SOON10", 10, "EUR"),
        CouponTerms {
            valid_from: Some(later),
            ..Default::default()
        },
    )
    .await;

    let refused = refusal(&env, coupon_id).await.expect("refused");
    assert!(refused.contains("not valid yet"), "{refused}");
}

#[rstest]
#[tokio::test]
async fn a_fixed_coupon_with_no_amount_in_the_currency_is_refused(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let coupon_id = create(
        &env,
        fixed("USDONLY", 10, "USD"),
        CouponTerms {
            fixed_amounts: BTreeMap::from([("GBP".to_owned(), Decimal::from(8))]),
            ..Default::default()
        },
    )
    .await;

    let refused = refusal(&env, coupon_id).await.expect("refused");
    assert!(refused.contains("has no amount in EUR"), "{refused}");
}

/// A USD coupon priced 10 EUR too takes 10 EUR off an EUR subscription.
#[rstest]
#[tokio::test]
async fn a_fixed_coupon_takes_its_amount_in_the_subscription_currency(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let coupon_id = create(
        &env,
        fixed("WORLD5", 5, "USD"),
        CouponTerms {
            fixed_amounts: BTreeMap::from([("EUR".to_owned(), Decimal::from(10))]),
            ..Default::default()
        },
    )
    .await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(4900)
        .has_discount(1000)
        .has_total(3900)
        .has_coupons_count(1);
}

/// A redemption keeps the amount it was granted: the amount in its currency cannot be removed
/// while it is active, and a changed amount leaves the redemption's as it was.
#[rstest]
#[tokio::test]
async fn a_redemption_keeps_its_fixed_amount(#[future] test_env: TestEnv) {
    use meteroid_store::domain::coupons::CouponPatch;
    use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;

    let env = test_env.await;
    let coupon_id = create(
        &env,
        fixed("WORLD5", 5, "USD"),
        CouponTerms {
            fixed_amounts: BTreeMap::from([("EUR".to_owned(), Decimal::from(10))]),
            ..Default::default()
        },
    )
    .await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .on_start()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    let patch = |fixed_amounts: BTreeMap<String, Decimal>| CouponPatch {
        id: coupon_id,
        tenant_id: TENANT_ID,
        description: None,
        discount: None,
        plan_ids: None,
        add_on_ids: None,
        eligibility: None,
        valid_from: None,
        expires_at: None,
        fixed_amounts: Some(fixed_amounts),
        duration_months: None,
    };

    let refused = env
        .store()
        .update_coupon(Actor::System, patch(BTreeMap::new()))
        .await
        .expect_err("refused");
    assert!(
        format!("{refused:?}").contains("redeemed on active subscriptions in EUR"),
        "{refused:?}"
    );
    env.store()
        .update_coupon(
            Actor::System,
            patch(BTreeMap::from([("EUR".to_owned(), Decimal::from(20))])),
        )
        .await
        .expect("amount changed");
    let details = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("subscription details");
    assert_eq!(
        details.applied_coupons[0].coupon.discount_in("EUR"),
        CouponDiscount::Fixed {
            currency: "EUR".to_owned(),
            amount: Decimal::from(10),
        }
    );
}

/// An operator applying a coupon still respects who the customer is: a custom property rule
/// refuses a customer without the property, and lets one holding it in.
#[rstest]
#[tokio::test]
async fn a_coupon_is_refused_to_a_customer_its_rule_leaves_out(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let coupon_id = create(
        &env,
        fixed("STUDENT10", 10, "EUR"),
        CouponTerms {
            eligibility: Some(Eligibility {
                properties: vec![PropertyMatch {
                    key: " student ".to_owned(),
                    values: vec!["verified".to_owned(), "verified".to_owned()],
                }],
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await;
    let stored = env
        .store()
        .get_coupon_by_id(TENANT_ID, coupon_id)
        .await
        .expect("coupon");
    assert_eq!(
        stored.eligibility.map(|rule| rule.properties),
        Some(vec![PropertyMatch {
            key: "student".to_owned(),
            values: vec!["verified".to_owned()],
        }])
    );

    let refused = refusal(&env, coupon_id).await.expect("refused");
    assert!(
        refused.contains("Your account does not qualify for this offer"),
        "{refused}"
    );

    {
        use diesel::{ExpressionMethods, QueryDsl};
        use diesel_async::RunQueryDsl;
        use diesel_models::schema::customer::dsl as c;
        let mut conn = env.conn().await;
        diesel::update(c::customer.filter(c::id.eq(CUST_UBER_ID)))
            .set(c::custom_properties.eq(serde_json::json!({ "student": "verified" })))
            .execute(&mut conn)
            .await
            .expect("property set");
    }
    assert_eq!(refusal(&env, coupon_id).await, None);
}

/// A plan's eligibility is set and cleared live: its versions stay as they are.
#[rstest]
#[tokio::test]
async fn a_plans_eligibility_is_set_and_cleared_live(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let before = env
        .store()
        .get_plan(
            PLAN_PAID_FREE_TRIAL_ID,
            TENANT_ID,
            PlanVersionFilter::Active,
        )
        .await
        .expect("plan");

    let rule = Eligibility {
        countries: vec!["br".to_owned()],
        ..Default::default()
    };
    let stored = env
        .store()
        .set_plan_eligibility(Actor::System, TENANT_ID, PLAN_PAID_FREE_TRIAL_ID, rule)
        .await
        .expect("set")
        .expect("a rule");
    assert_eq!(stored.countries, vec!["BR".to_owned()]);

    let after = env
        .store()
        .get_plan(
            PLAN_PAID_FREE_TRIAL_ID,
            TENANT_ID,
            PlanVersionFilter::Active,
        )
        .await
        .expect("plan");
    assert_eq!(after.plan.eligibility, Some(stored));
    assert_eq!(after.plan.active_version_id, before.plan.active_version_id);
    assert_eq!(after.plan.draft_version_id, before.plan.draft_version_id);
    let overview = env
        .store()
        .get_plan_overview(PLAN_PAID_FREE_TRIAL_ID, TENANT_ID)
        .await
        .expect("overview");
    assert!(
        overview
            .eligibility
            .is_some_and(|rule| rule.countries == ["BR"])
    );

    let cleared = env
        .store()
        .set_plan_eligibility(
            Actor::System,
            TENANT_ID,
            PLAN_PAID_FREE_TRIAL_ID,
            Eligibility::default(),
        )
        .await
        .expect("cleared");
    assert_eq!(cleared, None);
    let after = env
        .store()
        .get_plan(
            PLAN_PAID_FREE_TRIAL_ID,
            TENANT_ID,
            PlanVersionFilter::Active,
        )
        .await
        .expect("plan");
    assert_eq!(after.plan.eligibility, None);

    let refused = env
        .store()
        .set_plan_eligibility(
            Actor::System,
            TENANT_ID,
            PLAN_PAID_FREE_TRIAL_ID,
            Eligibility {
                countries: vec!["Narnia".to_owned()],
                ..Default::default()
            },
        )
        .await;
    assert!(refused.is_err());
}
