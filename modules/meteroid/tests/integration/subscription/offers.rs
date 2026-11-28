//! Offers (ADR 0019): what an offer writes on a subscription at its creation and once it starts,
//! what it withdraws when it never starts, and who is given it once. Starter is 29.00 + Seats 10.00
//! a month in advance, LeetCode 35.00; neither has a trial of its own.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CouponId, CreditGrantId, CustomerId, EntitlementEntityId, FeatureId, OfferGrantId,
    OfferId, PlanId, SubscriptionId,
};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::applied_coupons::{AppliedCouponDetailedRow, AppliedCouponRow};
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::offers::{OfferApplicationItemRow, OfferApplicationRow};
use meteroid_store::StoreResult;
use meteroid_store::domain::CreatedSubscription;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::credit::{CreditGrant, CreditGrantSource, GrantClassification};
use meteroid_store::domain::entitlement_fold::GrantValue;
use meteroid_store::domain::entitlements::{Entitlement, FeatureKind, FeatureNew};
use meteroid_store::domain::offers::{
    GrantAnchor, GrantCredit, GrantDefinition, GrantDiscount, GrantDuration, GrantEntitlement,
    GrantKind, GrantStatus, GrantTarget, GrantWindow, Offer, OfferGrant, OfferGrantNew, OfferNew,
    OfferRequest, OfferSource, OfferTrial, StartOfferChange, SubscriptionGrant,
};
use meteroid_store::domain::outbox_event::{CreditGrantUpdatedEvent, OutboxEvent};
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::trials::{TrialEligibility, TrialTerms};
use meteroid_store::errors::OfferRefusal;
use meteroid_store::repositories::coupons::CouponInterfaceAuto;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::entitlements::EntitlementsInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::services::CheckoutPaymentOutcome;
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::phases::{at, day};
use crate::data::ids::*;
use crate::harness::{
    InvoicesAssertExt, SubscriptionAssertExt, SubscriptionBuilder, TestEnv, subscription, test_env,
};

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0).expect("midnight").and_utc()
}

fn discount(percent: Decimal, months: u32) -> OfferGrantNew {
    OfferGrantNew {
        window: GrantWindow {
            anchor: GrantAnchor::BillingStart,
            duration: GrantDuration::Months(months),
        },
        definition: GrantDefinition::Discount(GrantDiscount {
            discount: CouponDiscount::Percentage(percent),
            fixed_amounts: BTreeMap::new(),
            add_on_ids: vec![],
        }),
    }
}

fn credit(amount: Decimal, days: u32) -> OfferGrantNew {
    OfferGrantNew {
        window: GrantWindow {
            anchor: GrantAnchor::Start,
            duration: GrantDuration::Days(days),
        },
        definition: GrantDefinition::Credit(GrantCredit {
            currency: "EUR".to_string(),
            amount,
            scope: Default::default(),
        }),
    }
}

fn entitlement(feature_id: FeatureId, duration: GrantDuration) -> OfferGrantNew {
    OfferGrantNew {
        window: GrantWindow {
            anchor: GrantAnchor::Start,
            duration,
        },
        definition: GrantDefinition::Entitlement(GrantEntitlement {
            feature_id,
            value: GrantValue::Boolean(true),
            effect: None,
        }),
    }
}

async fn feature(env: &TestEnv, code: &str) -> FeatureId {
    env.store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: code.into(),
                code: code.into(),
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

async fn start_offer(
    env: &TestEnv,
    plan: PlanId,
    trial: Option<TrialTerms>,
    grants: Vec<OfferGrantNew>,
) -> Offer {
    env.services()
        .set_plan_start_offer(
            Actor::User { id: USER_ID },
            TENANT_ID,
            plan,
            StartOfferChange::Inline {
                trial,
                grants,
                eligibility: None,
            },
        )
        .await
        .expect("the plan's start offer")
        .expect("a start offer")
        .offer
}

async fn catalog_offer(
    env: &TestEnv,
    trial: Option<OfferTrial>,
    grants: Vec<OfferGrantNew>,
    eligibility: TrialEligibility,
) -> Offer {
    env.services()
        .create_offer(
            TENANT_ID,
            OfferNew {
                name: "Welcome".to_string(),
                description: None,
                trial,
                grants,
                eligibility,
            },
        )
        .await
        .expect("a catalog offer")
}

fn grant_of(offer: &Offer, kind: GrantKind) -> &OfferGrant {
    offer
        .grants
        .iter()
        .find(|grant| grant.kind() == kind)
        .unwrap_or_else(|| panic!("a {kind:?} grant: {:?}", offer.grants))
}

fn given(grants: &[SubscriptionGrant], kind: GrantKind) -> &SubscriptionGrant {
    grants
        .iter()
        .find(|grant| grant.grant.kind() == kind)
        .unwrap_or_else(|| panic!("a {kind:?} grant given: {grants:?}"))
}

fn starter() -> SubscriptionBuilder {
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
}

fn pending_checkout() -> SubscriptionBuilder {
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .no_trial()
        .auto_charge()
}

async fn create(env: &TestEnv, builder: SubscriptionBuilder) -> StoreResult<CreatedSubscription> {
    env.services()
        .insert_subscription(Actor::User { id: USER_ID }, builder.request(), TENANT_ID)
        .await
}

fn refusal<T: std::fmt::Debug>(result: StoreResult<T>) -> Option<OfferRefusal> {
    OfferRefusal::of(&result.expect_err("refused"))
}

async fn applications(env: &TestEnv, sub_id: SubscriptionId) -> Vec<OfferApplicationRow> {
    let mut conn = env.conn().await;
    OfferApplicationRow::list_by_subscriptions(&mut conn, &TENANT_ID, &[sub_id])
        .await
        .expect("the offers applied")
}

async fn only_application(env: &TestEnv, sub_id: SubscriptionId) -> OfferApplicationRow {
    let mut applied = applications(env, sub_id).await;
    assert_eq!(applied.len(), 1, "one application: {applied:?}");
    applied.remove(0)
}

async fn items(env: &TestEnv, sub_id: SubscriptionId) -> Vec<OfferApplicationItemRow> {
    let ids: Vec<_> = applications(env, sub_id)
        .await
        .iter()
        .map(|a| a.id)
        .collect();
    let mut conn = env.conn().await;
    OfferApplicationItemRow::list_by_applications(&mut conn, &ids)
        .await
        .expect("the rows offers wrote")
}

/// Each item's window end (`ends_at`), by its grant and kind.
async fn window_ends(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> BTreeMap<(OfferGrantId, String), Option<NaiveDateTime>> {
    items(env, sub_id)
        .await
        .into_iter()
        .map(|i| ((i.grant_id, i.kind), i.ends_at))
        .collect()
}

fn ends_on(day: NaiveDate) -> Option<NaiveDateTime> {
    Some(midnight(day).naive_utc())
}

async fn credit_grant_updates(env: &TestEnv, grant: CreditGrantId) -> Vec<CreditGrantUpdatedEvent> {
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("webhook queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::CreditGrantUpdated(updated) if updated.credit_grant_id == grant => {
                Some(*updated)
            }
            _ => None,
        })
        .collect()
}

async fn applied_coupons(env: &TestEnv, sub_id: SubscriptionId) -> Vec<AppliedCouponRow> {
    let mut conn = env.conn().await;
    AppliedCouponDetailedRow::list_by_subscription_id(&mut conn, &sub_id)
        .await
        .expect("applied coupons")
        .into_iter()
        .map(|row| row.applied_coupon)
        .collect()
}

async fn promo_grants(env: &TestEnv, customer_id: CustomerId) -> Vec<CreditGrant> {
    env.store()
        .customer_credit_wallets(TENANT_ID, customer_id)
        .await
        .expect("wallets")
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == CreditGrantSource::Coupon)
        .collect()
}

async fn entitlements(env: &TestEnv, sub_id: SubscriptionId) -> Vec<Entitlement> {
    env.store()
        .list_entitlements_by_entity(EntitlementEntityId::Subscription(sub_id), TENANT_ID)
        .await
        .expect("the subscription's entitlements")
}

async fn grants(env: &TestEnv, sub_id: SubscriptionId) -> Vec<SubscriptionGrant> {
    env.services()
        .subscription_grants(TENANT_ID, sub_id)
        .await
        .expect("the subscription's grants")
}

async fn checkout(env: &TestEnv, sub_id: SubscriptionId, amount: u64) {
    let mut conn = env.conn().await;
    let (_, outcome) = env
        .services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            amount,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout");
    assert_eq!(outcome, CheckoutPaymentOutcome::Settled);
}

/// A v1 percentage coupon is wrapped in an offer of its own; redeemed at creation it writes the
/// applied coupon it wrote before (10% of 39.00 billed), and an application of its offer.
/// Rules: R-174
#[rstest]
#[tokio::test]
async fn a_v1_coupon_redeemed_at_creation_writes_what_it_wrote_before(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let coupon_id = env.create_percentage_coupon("TEN", 10).await;
    let wrapper = env
        .store()
        .get_coupon_by_id(TENANT_ID, coupon_id)
        .await
        .expect("the coupon")
        .offer_id
        .expect("wrapped in an offer of its own");
    let offer = env
        .services()
        .find_offer(TENANT_ID, wrapper)
        .await
        .expect("its offer");
    assert!(!offer.standalone);
    assert_eq!(offer.trial, None);
    assert_eq!(offer.eligibility, TrialEligibility::None);
    assert_eq!(
        offer
            .grants
            .iter()
            .map(|grant| (grant.kind(), grant.coupon_id))
            .collect::<Vec<_>>(),
        vec![(GrantKind::Discount, Some(coupon_id))]
    );

    let sub_id = starter()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(
        coupons
            .iter()
            .map(|c| (c.coupon_id, c.is_active, c.effective_from))
            .collect::<Vec<_>>(),
        vec![(coupon_id, true, day(1, 1))]
    );
    let application = only_application(&env, sub_id).await;
    assert_eq!(
        (
            application.offer_id,
            application.coupon_id,
            application.source.as_str()
        ),
        (wrapper, Some(coupon_id), "COUPON")
    );
    assert_eq!(
        items(&env, sub_id)
            .await
            .iter()
            .map(|i| (i.kind.as_str(), i.target_id))
            .collect::<Vec<_>>(),
        vec![("APPLIED_COUPON", coupons[0].id.as_uuid())]
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_subtotal(3900)
        .has_discount(390);
}

/// A v1 credit coupon redeemed at creation funds the one grant it funded before: 25.00 for 30
/// days, its source the applied coupon, its reason and reference the code.
/// Rules: R-174
#[rstest]
#[tokio::test]
async fn a_v1_credit_coupon_funds_the_grant_it_funded_before(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let coupon_id = env
        .create_coupon(CouponNew {
            code: "WELCOME25".to_owned(),
            description: "Welcome offer".to_owned(),
            tenant_id: TENANT_ID,
            discount: CouponDiscount::Credit {
                currency: "EUR".to_owned(),
                amount: dec!(25),
                expires_in_days: Some(30),
                scope: Default::default(),
            },
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: false,
            plan_ids: vec![],
        })
        .await;
    let wrapper = env
        .store()
        .get_coupon_by_id(TENANT_ID, coupon_id)
        .await
        .expect("the coupon")
        .offer_id
        .expect("wrapped in an offer of its own");
    let offer = env
        .services()
        .find_offer(TENANT_ID, wrapper)
        .await
        .expect("its offer");
    let grant = grant_of(&offer, GrantKind::Credit);
    assert_eq!(grant.coupon_id, Some(coupon_id));
    assert_eq!(
        grant.window,
        GrantWindow {
            anchor: GrantAnchor::Start,
            duration: GrantDuration::Days(30),
        }
    );

    let sub_id = starter()
        .no_trial()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(
        coupons
            .iter()
            .map(|c| (c.coupon_id, c.is_active, c.effective_from))
            .collect::<Vec<_>>(),
        vec![(coupon_id, true, day(1, 1))]
    );
    let funded = promo_grants(&env, CUST_UBER_ID).await;
    assert_eq!(funded.len(), 1, "one grant: {funded:?}");
    let funded = &funded[0];
    assert_eq!(funded.amount_minor, 2_500);
    assert_eq!(funded.classification, GrantClassification::Promotional);
    assert_eq!(funded.source_id, coupons[0].id.as_uuid());
    assert_eq!(funded.reason, "Promo code WELCOME25");
    assert_eq!(funded.reference.as_deref(), Some("WELCOME25"));
    assert_eq!(funded.effective_at.date_naive(), day(1, 1));
    assert_eq!(
        funded.expires_at.map(|end| end - funded.effective_at),
        Some(chrono::Duration::days(30))
    );
    let application = only_application(&env, sub_id).await;
    assert_eq!(
        (
            application.offer_id,
            application.coupon_id,
            application.source.as_str()
        ),
        (wrapper, Some(coupon_id), "COUPON")
    );
}

/// Starter's start offer: 20% for three months of billing, 25.00 of credit for 30 days and SSO
/// for 30 days. Created started on January 1, each is written in its engine and given at once.
/// Rules: R-171, R-174
#[rstest]
#[tokio::test]
async fn each_grant_kind_is_written_in_its_engine(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sso = feature(&env, "sso").await;
    let offer = start_offer(
        &env,
        PLAN_STARTER_ID,
        None,
        vec![
            discount(dec!(20), 3),
            credit(dec!(25), 30),
            entitlement(sso, GrantDuration::Days(30)),
        ],
    )
    .await;
    let discount_coupon = grant_of(&offer, GrantKind::Discount)
        .coupon_id
        .expect("its coupon");
    let credit_coupon = grant_of(&offer, GrantKind::Credit)
        .coupon_id
        .expect("its coupon");

    let sub_id = starter().create(env.services()).await;

    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(coupons.len(), 2, "{coupons:?}");
    assert!(
        coupons
            .iter()
            .all(|c| c.is_active && c.effective_from == day(1, 1)),
        "{coupons:?}"
    );
    let applied = |coupon: CouponId| {
        coupons
            .iter()
            .find(|c| c.coupon_id == coupon)
            .expect("its applied coupon")
            .id
    };
    let (discount_applied, credit_applied) = (applied(discount_coupon), applied(credit_coupon));

    let funded = promo_grants(&env, CUST_UBER_ID).await;
    assert_eq!(funded.len(), 1, "{funded:?}");
    assert_eq!(funded[0].amount_minor, 2_500);
    assert_eq!(funded[0].source_id, credit_applied.as_uuid());
    assert_eq!(funded[0].effective_at.date_naive(), day(1, 1));
    assert_eq!(
        funded[0].expires_at.map(|end| end - funded[0].effective_at),
        Some(chrono::Duration::days(30))
    );

    let rows = entitlements(&env, sub_id).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].feature_id, sso);
    assert_eq!(
        (rows[0].valid_from, rows[0].valid_until),
        (Some(midnight(day(1, 1))), Some(midnight(day(1, 31))))
    );

    let given_grants = grants(&env, sub_id).await;
    assert_eq!(given_grants.len(), 3, "{given_grants:?}");
    assert!(
        given_grants.iter().all(|g| g.status == GrantStatus::Active
            && g.application.source == OfferSource::Plan
            && g.application.offer_id == offer.id),
        "{given_grants:?}"
    );
    let discounted = given(&given_grants, GrantKind::Discount);
    assert_eq!(
        discounted.targets,
        vec![GrantTarget::AppliedCoupon(discount_applied)]
    );
    assert_eq!(
        (discounted.from, discounted.until),
        (day(1, 1), Some(day(4, 1)))
    );
    let credited = &given(&given_grants, GrantKind::Credit).targets;
    assert_eq!(credited.len(), 2, "{credited:?}");
    assert!(credited.contains(&GrantTarget::AppliedCoupon(credit_applied)));
    assert!(credited.contains(&GrantTarget::CreditGrant(funded[0].id)));
    assert_eq!(
        given(&given_grants, GrantKind::Entitlement).targets,
        vec![GrantTarget::Entitlement(rows[0].id)]
    );

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_subtotal(3900)
        .has_discount(780);
}

/// Awaiting its checkout, the subscription has its credit's applied coupon only; paid on
/// January 3, its credit is funded from then and its entitlement written, once.
/// Rules: R-174
#[rstest]
#[tokio::test]
async fn a_checkout_completed_later_gives_the_rest_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 9));
    let sso = feature(&env, "sso").await;
    let offer = start_offer(
        &env,
        PLAN_LEETCODE_ID,
        None,
        vec![
            credit(dec!(25), 30),
            entitlement(sso, GrantDuration::Days(30)),
        ],
    )
    .await;
    let credit_coupon = grant_of(&offer, GrantKind::Credit)
        .coupon_id
        .expect("its coupon");

    let sub_id = pending_checkout().create(env.services()).await;
    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(
        coupons
            .iter()
            .map(|c| (c.coupon_id, c.is_active))
            .collect::<Vec<_>>(),
        vec![(credit_coupon, true)]
    );
    assert!(promo_grants(&env, CUST_UBER_ID).await.is_empty());
    assert!(entitlements(&env, sub_id).await.is_empty());
    let application = only_application(&env, sub_id).await;
    assert_eq!(application.started_at, None);
    assert!(
        grants(&env, sub_id)
            .await
            .iter()
            .all(|g| g.status == GrantStatus::Scheduled)
    );

    env.set_now(at(1, 3, 9));
    checkout(&env, sub_id, 3500).await;
    env.get_subscription(sub_id).await.assert().is_active();
    let funded = promo_grants(&env, CUST_UBER_ID).await;
    assert_eq!(funded.len(), 1, "{funded:?}");
    assert_eq!(funded[0].source_id, coupons[0].id.as_uuid());
    assert_eq!(funded[0].effective_at.date_naive(), day(1, 3));
    assert_eq!(
        funded[0].expires_at.map(|end| end - funded[0].effective_at),
        Some(chrono::Duration::days(30))
    );
    let rows = entitlements(&env, sub_id).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].valid_from, rows[0].valid_until),
        (Some(midnight(day(1, 1))), Some(midnight(day(1, 31))))
    );
    let application = only_application(&env, sub_id).await;
    assert!(application.started_at.is_some());
    assert_eq!(items(&env, sub_id).await.len(), 3);

    env.process_cycles().await;
    env.run_outbox_and_orchestration().await;
    assert_eq!(promo_grants(&env, CUST_UBER_ID).await.len(), 1);
    assert_eq!(entitlements(&env, sub_id).await.len(), 1);
    assert_eq!(
        items(&env, sub_id).await.len(),
        3,
        "a replay writes nothing"
    );
}

/// A checkout never completed, cancelled on January 5: the subscription is aborted and loses what
/// its offer gave; withdrawn, the offer is given to the customer's next subscription.
/// Rules: R-173, R-174
#[rstest]
#[tokio::test]
async fn an_abandoned_checkout_withdraws_what_its_offer_gave(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 9));
    let sso = feature(&env, "sso").await;
    let offer = start_offer(
        &env,
        PLAN_LEETCODE_ID,
        None,
        vec![
            discount(dec!(20), 3),
            credit(dec!(25), 30),
            entitlement(sso, GrantDuration::Days(30)),
        ],
    )
    .await;
    let sub_id = pending_checkout().create(env.services()).await;

    env.set_now(at(1, 5, 9));
    env.services()
        .cancel_subscription(
            Actor::User { id: USER_ID },
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled");
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Aborted);
    let application = only_application(&env, sub_id).await;
    assert!(application.withdrawn_at.is_some());
    let written = items(&env, sub_id).await;
    assert_eq!(written.len(), 2, "{written:?}");
    assert!(written.iter().all(|i| i.ended_at.is_some()), "{written:?}");
    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(coupons.len(), 2, "{coupons:?}");
    assert!(coupons.iter().all(|c| !c.is_active), "{coupons:?}");
    assert!(promo_grants(&env, CUST_UBER_ID).await.is_empty());
    assert!(entitlements(&env, sub_id).await.is_empty());
    let withdrawn = grants(&env, sub_id).await;
    assert_eq!(withdrawn.len(), 3, "{withdrawn:?}");
    assert!(
        withdrawn.iter().all(|g| g.status == GrantStatus::Withdrawn),
        "{withdrawn:?}"
    );

    let next = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 5))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let application = only_application(&env, next).await;
    assert_eq!(
        (application.offer_id, application.source.as_str()),
        (offer.id, "PLAN")
    );
}

/// A plan's start offer is given once per product family: asked by default again, the customer's
/// second subscription gets nothing. A catalog offer given once, named again, is refused.
/// Rules: R-173
#[rstest]
#[tokio::test]
async fn an_offer_is_given_once_per_customer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let offer = start_offer(&env, PLAN_STARTER_ID, None, vec![discount(dec!(20), 3)]).await;
    assert_eq!(offer.eligibility, TrialEligibility::ProductFamily);

    let first = starter().create(env.services()).await;
    let application = only_application(&env, first).await;
    assert_eq!(
        (application.offer_id, application.source.as_str()),
        (offer.id, "PLAN")
    );
    let second = starter().create(env.services()).await;
    assert!(applications(&env, second).await.is_empty());
    assert!(applied_coupons(&env, second).await.is_empty());

    let welcome = catalog_offer(
        &env,
        None,
        vec![discount(dec!(10), 1)],
        TrialEligibility::Offer,
    )
    .await;
    let third = create(&env, starter().offer(OfferRequest::Offer(welcome.id)))
        .await
        .expect("given once")
        .id;
    let application = only_application(&env, third).await;
    assert_eq!(
        (application.offer_id, application.source.as_str()),
        (welcome.id, "OPERATOR")
    );
    let again = create(&env, starter().offer(OfferRequest::Offer(welcome.id))).await;
    assert_eq!(refusal(again), Some(OfferRefusal::AlreadyGiven));
}

/// An offer applied to a running subscription is applied once; one with a trial is not applied
/// after the subscription's start.
/// Rules: R-173, R-174
#[rstest]
#[tokio::test]
async fn an_offer_is_applied_once_to_a_running_subscription(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = starter().create(env.services()).await;
    let loyalty = catalog_offer(
        &env,
        None,
        vec![discount(dec!(10), 1)],
        TrialEligibility::None,
    )
    .await;
    let apply = |offer: OfferId| {
        let env = &env;
        async move {
            env.services()
                .apply_offer(
                    Actor::User { id: USER_ID },
                    TENANT_ID,
                    sub_id,
                    offer,
                    OfferSource::Operator,
                )
                .await
        }
    };

    let applied = apply(loyalty.id).await.expect("applied");
    assert_eq!(applied.len(), 1, "{applied:?}");
    assert_eq!(applied[0].application.offer_id, loyalty.id);
    assert_eq!(applied[0].application.source, OfferSource::Operator);
    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(
        coupons.iter().map(|c| c.coupon_id).collect::<Vec<_>>(),
        vec![loyalty.grants[0].coupon_id.expect("its coupon")]
    );
    assert_eq!(
        applied[0].targets,
        vec![GrantTarget::AppliedCoupon(coupons[0].id)]
    );

    assert_eq!(
        refusal(apply(loyalty.id).await),
        Some(OfferRefusal::AlreadyApplied)
    );
    assert_eq!(applied_coupons(&env, sub_id).await.len(), 1);

    let trial = catalog_offer(
        &env,
        Some(OfferTrial::from_terms(TrialTerms::days(14, None))),
        vec![],
        TrialEligibility::None,
    )
    .await;
    assert_eq!(
        refusal(apply(trial.id).await),
        Some(OfferRefusal::TrialNotApplicable)
    );
}

/// Starter's start offer: two weeks free, then 20% for three months of billing, and SSO until
/// billing starts. The trial is the plan's as before; asking no offer gives neither.
/// Rules: R-170, R-172, R-174
#[rstest]
#[tokio::test]
async fn a_start_offer_with_a_trial_gives_it_and_its_grants(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sso = feature(&env, "sso").await;
    let offer = start_offer(
        &env,
        PLAN_STARTER_ID,
        Some(TrialTerms::days(14, None)),
        vec![
            discount(dec!(20), 3),
            entitlement(sso, GrantDuration::UntilBillingStarts),
        ],
    )
    .await;
    assert_eq!(offer.trial.map(|trial| trial.days), Some(14));

    let sub_id = starter().create(env.services()).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active();
    assert_eq!(env.trial_days(sub_id).await, Some(14));
    let coupons = applied_coupons(&env, sub_id).await;
    assert_eq!(
        coupons
            .iter()
            .map(|c| (c.coupon_id, c.is_active, c.effective_from))
            .collect::<Vec<_>>(),
        vec![(
            grant_of(&offer, GrantKind::Discount)
                .coupon_id
                .expect("its coupon"),
            true,
            day(1, 15)
        )]
    );
    let rows = entitlements(&env, sub_id).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].valid_from, rows[0].valid_until),
        (Some(midnight(day(1, 1))), Some(midnight(day(1, 15))))
    );
    let given_grants = grants(&env, sub_id).await;
    let discounted = given(&given_grants, GrantKind::Discount);
    assert_eq!(
        (discounted.status, discounted.from, discounted.until),
        (GrantStatus::Scheduled, day(1, 15), Some(day(4, 15)))
    );
    let featured = given(&given_grants, GrantKind::Entitlement);
    assert_eq!(
        (featured.status, featured.from, featured.until),
        (GrantStatus::Active, day(1, 1), Some(day(1, 15)))
    );

    let without = starter()
        .customer(CUST_SPOTIFY_ID)
        .offer(OfferRequest::None)
        .create(env.services())
        .await;
    assert_eq!(env.trial_days(without).await, None);
    assert!(applications(&env, without).await.is_empty());
    assert!(applied_coupons(&env, without).await.is_empty());
    assert!(entitlements(&env, without).await.is_empty());
}

/// An import asks no offer, as the CSV import and the cutover do: the plan's start offer gives it
/// nothing.
/// Rules: R-170
#[rstest]
#[tokio::test]
async fn an_imported_subscription_is_given_no_offer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(3, 10, 12));
    start_offer(
        &env,
        PLAN_STARTER_ID,
        Some(TrialTerms::days(14, None)),
        vec![discount(dec!(20), 3)],
    )
    .await;

    let sub_id = starter()
        .offer(OfferRequest::None)
        .skip_past_invoices()
        .create(env.services())
        .await;
    assert!(applications(&env, sub_id).await.is_empty());
    assert!(applied_coupons(&env, sub_id).await.is_empty());
    assert!(grants(&env, sub_id).await.is_empty());
    assert_eq!(env.trial_days(sub_id).await, None);
}

/// LeetCode's start offer takes 20% off for 3 months: its checkout previews 28.00 due, the 7.00
/// off read by the offer's name, and completing it at 28.00 bills the first invoice 28.00.
/// Rules: R-176
#[rstest]
#[tokio::test]
async fn a_checkout_previews_the_start_offer_as_it_bills_it(#[future] test_env: TestEnv) {
    use meteroid_store::domain::CreateCheckoutSession;
    use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 9));
    let offer = start_offer(&env, PLAN_LEETCODE_ID, None, vec![discount(dec!(20), 3)]).await;
    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            option: None,
            billing_start_date: Some(day(1, 1)),
            billing_day_anchor: Some(1),
            net_terms: None,
            offer: OfferRequest::Plan,
            trial: meteroid_store::domain::trials::TrialRequest::None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
            resource_id: None,
        })
        .await
        .expect("a checkout session");

    let details = env
        .services()
        .build_preview_subscription_details(&session, TENANT_ID, None)
        .await
        .expect("a preview");
    let grant_coupon = grant_of(&offer, GrantKind::Discount)
        .coupon_id
        .expect("its coupon");
    assert_eq!(
        details
            .applied_coupons
            .iter()
            .map(|applied| (applied.coupon.id, applied.coupon.is_grant_definition()))
            .collect::<Vec<_>>(),
        vec![(grant_coupon, true)]
    );
    let previewed = env
        .services()
        .compute_new_subscription_invoice(&details)
        .await
        .expect("the preview's invoice");
    assert_eq!(
        (previewed.subtotal, previewed.discount, previewed.amount_due),
        (3500, 700, 2800)
    );
    assert_eq!(previewed.applied_coupons[0].name, offer.name);

    let completed = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            2800,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout completes at the amount previewed");
    let subscription_id = match &completed {
        CheckoutCompletionResult::Completed {
            subscription_id, ..
        } => *subscription_id,
        _ => panic!("the mock provider settles at once: {completed:?}"),
    };
    let invoices = env.get_invoices(subscription_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_subtotal(3500)
        .has_discount(700)
        .has_total(2800);
    assert_eq!(invoices[0].coupons[0].name, offer.name);
}

/// Starter's start offer gives a 14-day trial, 20% off for 3 months from billing's start, SSO
/// until billing starts and 25.00 credit for 30 days from billing's start. Extended by a week on
/// January 5, they all follow its end, January 22; ended now that day on another subscription,
/// they follow it to January 5.
/// Rules: R-177
#[rstest]
#[tokio::test]
async fn grants_bound_to_the_trial_follow_it_when_it_moves(#[future] test_env: TestEnv) {
    use meteroid_store::domain::trials::TrialMove;

    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sso = feature(&env, "sso").await;
    let mut billed_credit = credit(dec!(25), 30);
    billed_credit.window.anchor = GrantAnchor::BillingStart;
    let offer = start_offer(
        &env,
        PLAN_STARTER_ID,
        Some(TrialTerms::days(14, None)),
        vec![
            discount(dec!(20), 3),
            entitlement(sso, GrantDuration::UntilBillingStarts),
            billed_credit,
        ],
    )
    .await;
    let discount_coupon = grant_of(&offer, GrantKind::Discount)
        .coupon_id
        .expect("its coupon");
    let discounted_from = async |sub_id: SubscriptionId| {
        applied_coupons(&env, sub_id)
            .await
            .into_iter()
            .find(|c| c.coupon_id == discount_coupon)
            .expect("the discount's applied coupon")
            .effective_from
    };
    let extended = starter().create(env.services()).await;
    let ended = starter()
        .customer(CUST_SPOTIFY_ID)
        .create(env.services())
        .await;
    let funded = promo_grants(&env, CUST_UBER_ID).await;
    assert_eq!(funded.len(), 1, "{funded:?}");
    assert_eq!(funded[0].effective_at, midnight(day(1, 15)));
    let discount_grant = grant_of(&offer, GrantKind::Discount).id;
    let sso_grant = grant_of(&offer, GrantKind::Entitlement).id;
    let credit_grant = grant_of(&offer, GrantKind::Credit).id;
    let expected_ends = |discount: NaiveDate, sso: NaiveDate, credit: Option<NaiveDateTime>| {
        BTreeMap::from([
            (
                (discount_grant, "APPLIED_COUPON".to_string()),
                ends_on(discount),
            ),
            ((credit_grant, "APPLIED_COUPON".to_string()), None),
            ((credit_grant, "CREDIT_GRANT".to_string()), credit),
            ((sso_grant, "ENTITLEMENT".to_string()), ends_on(sso)),
        ])
    };
    assert_eq!(
        window_ends(&env, extended).await,
        expected_ends(day(4, 15), day(1, 15), ends_on(day(2, 14))),
        "each item's window end, the credit's as funded"
    );

    env.set_now(at(1, 5, 12));
    env.services()
        .move_trial(Actor::System, TENANT_ID, extended, TrialMove::Days(7))
        .await
        .expect("a week more, its grants with it");
    assert_eq!(
        only_application(&env, extended).await.billing_start_on,
        day(1, 22)
    );
    assert_eq!(discounted_from(extended).await, day(1, 22));
    let rows = entitlements(&env, extended).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        (rows[0].valid_from, rows[0].valid_until),
        (Some(midnight(day(1, 1))), Some(midnight(day(1, 22))))
    );
    let funded = promo_grants(&env, CUST_UBER_ID).await;
    assert_eq!(
        (funded[0].effective_at, funded[0].expires_at),
        (midnight(day(1, 22)), Some(midnight(day(2, 21))))
    );
    assert_eq!(
        window_ends(&env, extended).await,
        expected_ends(day(4, 22), day(1, 22), ends_on(day(2, 21))),
        "the window ends moved with the trial"
    );
    let updated = credit_grant_updates(&env, funded[0].id).await;
    assert_eq!(updated.len(), 1, "one credit_grant.updated: {updated:?}");
    assert_eq!(
        (updated[0].effective_at, updated[0].expires_at),
        (midnight(day(1, 22)), Some(midnight(day(2, 21))))
    );
    assert_eq!(updated[0].customer_id, CUST_UBER_ID);
    let given_grants = grants(&env, extended).await;
    let featured = given(&given_grants, GrantKind::Entitlement);
    assert_eq!(
        (featured.status, featured.until),
        (GrantStatus::Active, Some(day(1, 22)))
    );
    assert_eq!(
        given(&given_grants, GrantKind::Discount).until,
        Some(day(4, 22))
    );
    assert!(
        items(&env, extended)
            .await
            .iter()
            .any(|i| i.kind == "ENTITLEMENT" && i.target_id == rows[0].id.as_uuid()),
        "the item names the entitlement's lineage"
    );

    env.services()
        .move_trial(Actor::System, TENANT_ID, ended, TrialMove::Now)
        .await
        .expect("billed from today, its grants with it");
    let rows = entitlements(&env, ended).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].valid_until, Some(midnight(day(1, 5))));
    assert_eq!(discounted_from(ended).await, day(1, 5));
    let funded = promo_grants(&env, CUST_SPOTIFY_ID).await;
    assert_eq!(funded[0].effective_at.timestamp(), at(1, 5, 12).timestamp());
    let credit_expires = funded[0].expires_at.expect("an expiry");
    assert_eq!(
        credit_expires.timestamp(),
        (at(1, 5, 12) + chrono::Duration::days(30)).timestamp()
    );
    assert_eq!(
        window_ends(&env, ended).await,
        expected_ends(day(4, 5), day(1, 5), Some(credit_expires.naive_utc())),
        "the window ends moved to today"
    );
    let updated = credit_grant_updates(&env, funded[0].id).await;
    assert_eq!(updated.len(), 1, "one credit_grant.updated: {updated:?}");
    assert_eq!(
        (updated[0].effective_at, updated[0].expires_at),
        (funded[0].effective_at, funded[0].expires_at)
    );
}

/// Starter's start offer gives SSO for 30 days and 20% off for 3 months from January 1: on
/// February 1 the feature's window is over and `grant.ended` is published once for it, its item
/// ended on January 31; the discount's, on April 1.
/// Rules: R-178
#[rstest]
#[tokio::test]
async fn a_grant_whose_window_is_over_is_noticed_once(#[future] test_env: TestEnv) {
    use meteroid_store::services::grant_notices::notify_grant_windows_ended;

    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sso = feature(&env, "sso").await;
    start_offer(
        &env,
        PLAN_STARTER_ID,
        None,
        vec![
            discount(dec!(20), 3),
            entitlement(sso, GrantDuration::Days(30)),
        ],
    )
    .await;
    let sub_id = starter().no_trial().create(env.services()).await;
    let ends: BTreeMap<String, Option<NaiveDateTime>> = items(&env, sub_id)
        .await
        .into_iter()
        .map(|i| (i.kind, i.ends_at))
        .collect();
    assert_eq!(
        ends,
        BTreeMap::from([
            ("APPLIED_COUPON".to_string(), ends_on(day(4, 1))),
            ("ENTITLEMENT".to_string(), ends_on(day(1, 31))),
        ]),
        "each item keeps its window's end"
    );
    let store = env.store();

    let swept = async |now| {
        notify_grant_windows_ended(store, now)
            .await
            .expect("a sweep")
    };
    assert_eq!(swept(at(1, 30, 23)).await, 0);
    assert_eq!(swept(at(2, 1, 1)).await, 1);
    assert_eq!(swept(at(2, 1, 2)).await, 0, "noticed once");
    let ended: Vec<_> = items(&env, sub_id)
        .await
        .into_iter()
        .filter_map(|i| i.ended_at.map(|at| (i.kind, at)))
        .collect();
    assert_eq!(
        ended,
        vec![(
            "ENTITLEMENT".to_string(),
            day(1, 31).and_hms_opt(0, 0, 0).unwrap()
        )]
    );
    env.set_now(at(2, 1, 2));
    let given_grants = grants(&env, sub_id).await;
    assert_eq!(
        given(&given_grants, GrantKind::Entitlement).status,
        GrantStatus::Ended
    );
    assert_eq!(
        given(&given_grants, GrantKind::Discount).status,
        GrantStatus::Active
    );

    assert_eq!(swept(at(4, 1, 1)).await, 1);
    assert_eq!(swept(at(4, 2, 1)).await, 0);
}
