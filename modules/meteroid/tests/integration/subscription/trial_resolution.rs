//! A creation's trial, resolved once (R-153): the plan's trial asked by default, none, an offer or
//! its own terms; what a customer is given once (R-159). Starter is 29.00 + Seats 10.00 a month in
//! advance and has no trial of its own.

use common_domain::actor::Actor;
use meteroid_store::domain::trials::{
    TrialEligibility, TrialEndAction, TrialLength, TrialOfferNew, TrialRequest, TrialTerms,
    TrialWithoutCard,
};
use meteroid_store::repositories::PlansInterface;
use rstest::rstest;

use super::phases::{at, day, documents};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

/// The first invoice billing Starter's platform fee.
async fn first_platform_line(env: &TestEnv, sub: common_domain::ids::SubscriptionId) -> String {
    documents(env, sub)
        .await
        .into_iter()
        .find(|line| line.contains("Platform Fee") && !line.ends_with(" 0"))
        .expect("a platform fee billed")
}

async fn starter_trial(env: &TestEnv, days: u32, free: bool) {
    env.store()
        .set_plan_trial(
            Actor::System,
            TENANT_ID,
            PLAN_STARTER_ID,
            Some(TrialTerms {
                duration_days: days,
                free,
                trial_plan_id: None,
                card_required: None,
                end_action: None,
                without_card: None,
            }),
        )
        .await
        .expect("Starter's trial");
}

/// A trial of its own ending on January 20 bills from then, its periods on the 20th.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn a_trial_of_its_own_ends_on_its_date(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial(TrialRequest::Custom {
            length: TrialLength::Until(day(1, 20)),
            free: Some(true),
            trial_plan: None,
            manual: false,
        })
        .create(env.services())
        .await;
    env.run_until(day(1, 21)).await;
    assert_eq!(
        first_platform_line(&env, sub).await,
        "Recurring 2024-01-20: Platform Fee 2024-01-20..2024-02-20 2900"
    );
}

/// A trial is free: a billed one is refused as the plan's terms, as an offer and asked at a
/// creation, with the same message; a plan's terms written give a trial once per product family.
/// Rules: R-153, R-159
#[rstest]
#[tokio::test]
async fn a_billed_trial_is_refused_everywhere_it_is_asked(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let free = "a trial is free: a discounted start is a coupon or an introductory phase";
    let terms = env
        .store()
        .set_plan_trial(
            Actor::System,
            TENANT_ID,
            PLAN_STARTER_ID,
            Some(TrialTerms {
                free: false,
                ..TrialTerms::days(14, None)
            }),
        )
        .await;
    assert!(format!("{:?}", terms.expect_err("billed terms")).contains(free));
    let offer = env
        .services()
        .create_trial_offer(
            TENANT_ID,
            TrialOfferNew {
                name: "Billed".to_string(),
                code: None,
                duration_days: 14,
                free: false,
                trial_plan_id: None,
                card_required: true,
                end_action: TrialEndAction::Convert,
                without_card: TrialWithoutCard::AwaitPayment,
                eligibility: TrialEligibility::None,
            },
        )
        .await;
    assert!(format!("{:?}", offer.expect_err("a billed offer")).contains(free));
    let asked = env
        .services()
        .insert_subscription(
            Actor::System,
            subscription()
                .plan_version(PLAN_VERSION_STARTER_ID)
                .start_date(day(1, 1))
                .on_start()
                .trial(TrialRequest::Custom {
                    length: TrialLength::Days(7),
                    free: Some(false),
                    trial_plan: None,
                    manual: false,
                })
                .request(),
            TENANT_ID,
        )
        .await;
    assert!(format!("{:?}", asked.expect_err("a billed trial asked")).contains(free));

    starter_trial(&env, 14, true).await;
    let written = env
        .store()
        .plan_trial(TENANT_ID, PLAN_STARTER_ID)
        .await
        .expect("read")
        .expect("a trial");
    assert_eq!(
        (written.offer.free, written.offer.eligibility),
        (true, TrialEligibility::ProductFamily)
    );
}

/// A plan's trial edited after a subscription started never reaches it; one started afterwards
/// takes the new terms.
/// Rules: R-100, R-153
#[rstest]
#[tokio::test]
async fn a_plan_trial_edit_never_reaches_a_running_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    starter_trial(&env, 14, true).await;
    let starter = || {
        subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(day(1, 1))
            .on_start()
    };
    let before = starter().create(env.services()).await;
    starter_trial(&env, 21, true).await;
    let after = starter()
        .customer(CUST_SPOTIFY_ID)
        .create(env.services())
        .await;
    env.run_until(day(1, 23)).await;
    assert_eq!(
        first_platform_line(&env, before).await,
        "Recurring 2024-01-15: Platform Fee 2024-01-15..2024-02-15 2900"
    );
    assert_eq!(
        first_platform_line(&env, after).await,
        "Recurring 2024-01-22: Platform Fee 2024-01-22..2024-02-22 2900"
    );
}

/// An offer two plans share, edited from one of them, gives that plan a trial of its own: the
/// other keeps the offer as it was.
/// Rules: R-153
#[rstest]
#[tokio::test]
async fn editing_a_shared_trial_gives_the_plan_its_own(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let shared = env
        .services()
        .create_trial_offer(
            TENANT_ID,
            TrialOfferNew {
                name: "Two weeks".to_string(),
                code: Some("two_weeks".to_string()),
                duration_days: 14,
                free: true,
                trial_plan_id: None,
                card_required: true,
                end_action: TrialEndAction::Convert,
                without_card: TrialWithoutCard::AwaitPayment,
                eligibility: TrialEligibility::None,
            },
        )
        .await
        .expect("an offer")
        .id;
    for plan in [PLAN_STARTER_ID, PLAN_PRO_ID] {
        env.services()
            .set_plan_trial_offer(Actor::System, TENANT_ID, plan, Some(shared))
            .await
            .expect("its trial");
    }
    starter_trial(&env, 21, true).await;
    let trial_of = |plan| {
        let env = &env;
        async move {
            env.store()
                .plan_trial(TENANT_ID, plan)
                .await
                .expect("read")
                .expect("a trial")
        }
    };
    let starter = trial_of(PLAN_STARTER_ID).await;
    assert_ne!(starter.offer.id, shared);
    assert_eq!(starter.offer.duration_days, 21);
    let pro = trial_of(PLAN_PRO_ID).await;
    assert_eq!((pro.offer.id, pro.offer.duration_days), (shared, 14));
    assert_eq!(pro.plans, vec![PLAN_PRO_ID]);
}

/// A plan's trial given once per plan: the customer's first subscription gets it, the second,
/// asking nothing, none (billed from January 1); the plan's trial asked explicitly or the offer
/// named is refused.
/// Rules: R-159
#[rstest]
#[tokio::test]
async fn a_trial_given_once_per_plan_counts_what_the_customer_had(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let once = env
        .services()
        .create_trial_offer(
            TENANT_ID,
            TrialOfferNew {
                name: "Once".to_string(),
                code: None,
                duration_days: 14,
                free: true,
                trial_plan_id: None,
                card_required: true,
                end_action: TrialEndAction::Convert,
                without_card: TrialWithoutCard::AwaitPayment,
                eligibility: TrialEligibility::Plan,
            },
        )
        .await
        .expect("an offer")
        .id;
    env.services()
        .set_plan_trial_offer(Actor::System, TENANT_ID, PLAN_STARTER_ID, Some(once))
        .await
        .expect("Starter's trial");
    let starter = || {
        subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(day(1, 1))
            .on_start()
    };
    let first = starter().create(env.services()).await;
    let second = starter().create(env.services()).await;
    let asked = env
        .services()
        .insert_subscription(
            Actor::System,
            starter().trial(TrialRequest::Plan).request(),
            TENANT_ID,
        )
        .await;
    assert!(format!("{:?}", asked.expect_err("asked explicitly")).contains("already had"));
    let named = env
        .services()
        .insert_subscription(
            Actor::System,
            starter().trial_offer(once).request(),
            TENANT_ID,
        )
        .await;
    assert!(format!("{:?}", named.expect_err("given once")).contains("already had"));
    env.run_until(day(1, 16)).await;
    assert_eq!(
        first_platform_line(&env, first).await,
        "Recurring 2024-01-15: Platform Fee 2024-01-15..2024-02-15 2900"
    );
    assert_eq!(
        first_platform_line(&env, second).await,
        "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900"
    );
}

/// A coupon of one month on a free trial counts its month from the trial's end: 4900 less 50 % on
/// January 15, 4900 on February 15.
/// Rules: R-153, R-089
#[rstest]
#[tokio::test]
async fn a_coupon_counts_its_months_from_the_resolved_trial(#[future] test_env: TestEnv) {
    use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let coupon = env
        .create_coupon(CouponNew {
            code: "HALF1".to_string(),
            description: "50 % off a month".to_string(),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Percentage(rust_decimal::Decimal::from(50)),
            expires_at: None,
            redemption_limit: None,
            recurring_value: Some(1),
            reusable: false,
            plan_ids: vec![],
        })
        .await;
    let sub = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_start()
        .coupon(coupon)
        .create(env.services())
        .await;
    env.run_until(day(2, 16)).await;
    let invoices = env.get_invoices(sub).await;
    let billed: Vec<_> = invoices
        .iter()
        .filter(|invoice| invoice.subtotal > 0)
        .map(|invoice| {
            (
                invoice.billing_period_start,
                invoice.subtotal,
                invoice.discount,
            )
        })
        .collect();
    assert_eq!(
        billed,
        vec![(day(1, 15), 4900, 2450), (day(2, 15), 4900, 0)],
        "the coupon's month starts with billing"
    );
}
