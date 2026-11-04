//! S12. A promotion code redeemed in the portal by a customer with two subscriptions: the
//! customer-wide code is recorded on the oldest started one and pays both, scoped plan credit
//! first; lapsing, it settles the open usage of every subscription that can draw it. The
//! subscription-scoped variant must name its subscription and pays only that one.
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CouponId, CustomerId, SubscriptionId};
use diesel_async::RunQueryDsl;
use meteroid_store::domain::coupons::{CouponCreditScope, CouponDiscount, CouponNew};
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use rust_decimal_macros::dec;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};
use super::s10_two_subscriptions::keyed;

#[derive(diesel::QueryableByName)]
struct AppliedOn {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    subscription_id: uuid::Uuid,
}

/// A $100 promotional credit code expiring 25 days after it is redeemed, in the catalog all year.
async fn promo_code(h: &Halyard, code: &str, scope: CouponCreditScope) -> CouponId {
    h.services()
        .create_coupon_at(
            Actor::System,
            CouponNew {
                code: code.to_owned(),
                description: "Launch credit".to_owned(),
                tenant_id: h.tenant,
                discount: CouponDiscount::Credit {
                    currency: "USD".to_owned(),
                    amount: dec!(100),
                    expires_in_days: Some(25),
                    scope,
                },
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
            at(1, 1),
        )
        .await
        .expect("the promotion code")
        .id
}

/// A on Scale from Sep 1 (key-A, paid), B on Pay as you go from Sep 10 (key-B); A's September
/// usage 62000 (standard 50M + lite 10M), B's 9000 (standard 7.5M) before Oct 7.
async fn two_subscriptions(h: &Halyard) -> (CustomerId, SubscriptionId, SubscriptionId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let mut a_request = h.request(customer, h.scale, day(9, 1));
    a_request.subscription.usage_filter = keyed("key-A");
    let (a, a1) = h.start(a_request).await;
    h.pay(a1.expect("INV-A1").id, at(9, 1)).await;
    let mut b_request = h.request(customer, h.payg, day(9, 10));
    b_request.subscription.usage_filter = keyed("key-B");
    let (b, _) = h.start(b_request).await;
    h.keyed_tokens(customer, at_time(9, 3, 10, 0, 0), STANDARD, 50 * M, "key-A");
    h.keyed_tokens(customer, at_time(9, 20, 10, 0, 0), LITE, 10 * M, "key-A");
    h.keyed_tokens(customer, at_time(9, 15, 10, 0, 0), STANDARD, 5 * M, "key-B");
    h.keyed_tokens(
        customer,
        at_time(9, 28, 10, 0, 0),
        STANDARD,
        5 * M / 2,
        "key-B",
    );
    h.check().await;
    (customer, a, b)
}

#[tokio::test]
async fn s12_a_promo_code_redeemed_with_two_subscriptions() {
    let h = Halyard::new().await;
    let launch = promo_code(&h, "LAUNCH100", CouponCreditScope::Customer).await;
    let (customer, a, b) = two_subscriptions(&h).await;

    let redeemed = h
        .services()
        .redeem_coupon_credit_code_at(h.tenant, customer, "launch100", at(9, 12))
        .await
        .expect("redeemed in the portal");
    assert_eq!(
        (redeemed.amount_minor, redeemed.expires_at),
        (10_000, Some(at(10, 7))),
        "the grant instant + 25 days (OQ20)"
    );
    let applied: AppliedOn =
        diesel::sql_query("SELECT subscription_id FROM applied_coupon WHERE coupon_id = $1")
            .bind::<diesel::sql_types::Uuid, _>(launch.as_uuid())
            .get_result(&mut h.env.conn().await)
            .await
            .expect("the applied coupon");
    assert_eq!(applied.subscription_id, a.as_uuid(), "the oldest started");
    let again = h
        .services()
        .redeem_coupon_credit_code_at(h.tenant, customer, "LAUNCH100", at(9, 12))
        .await
        .expect_err("redeemed once per customer");
    assert!(
        format!("{again:?}").contains("You've already redeemed this code."),
        "{again:?}"
    );
    let unknown = h
        .services()
        .redeem_coupon_credit_code_at(h.tenant, customer, "NOPE", at(9, 12))
        .await
        .expect_err("no such code");
    assert!(
        format!("{unknown:?}").contains("This code can't be redeemed on your account."),
        "{unknown:?}"
    );
    let grants = h.grants(customer).await;
    let promo = grant(&grants, "COUPON", "PROMOTIONAL", 10_000);
    assert_eq!(
        (
            promo.subscription_id,
            promo.application.as_str(),
            promo.expires_at
        ),
        (None, "INVOICE", Some(at(10, 7)))
    );
    let promo = promo.id;
    h.check().await;

    let a2 = h.renew(a, at(10, 1)).await.expect("INV-A2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let a2 = h.invoice(a2.id).await;
    assert_eq!(
        money(&a2),
        (112_000, 0, 12_000, 0, 100_000, 50_000, 50_000),
        "B 5000 + W 5000 + G-promo 2000: subscription-scoped credit before the promotion: {:?}",
        lines(&a2)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        0
    );
    assert_eq!(
        grants.iter().find(|g| g.id == promo).unwrap().remaining,
        8_000
    );
    h.pay(a2.id, at(10, 1)).await;
    h.check().await;

    // Oct 7: the promotion lapses and settles B's open usage (A has none in October).
    h.sweep(customer, at(10, 7) + Halyard::hours(1)).await;
    assert_eq!(
        h.movements(promo).await,
        vec!["FUND 10000", "DRAW -2000", "DRAW -8000"],
        "settles 8000 of B's 9000, nothing left to lapse"
    );
    h.check().await;

    let b1 = h.renew(b, at(10, 10)).await.expect("INV-B1");
    let b1 = h.invoice(b1.id).await;
    assert_eq!(
        money(&b1),
        (9_000, 0, 8_000, 0, 1_000, 0, 1_000),
        "{:?}",
        lines(&b1)
    );
    h.check().await;
}

/// LAUNCH-SUB: a subscription-scoped code needs its subscription named; redeemed for B, it pays
/// B's usage only and lapses with what B did not use.
#[tokio::test]
async fn s12_launch_sub_a_subscription_scoped_code_pays_its_subscription_only() {
    let h = Halyard::new().await;
    promo_code(&h, "LAUNCHSUB", CouponCreditScope::Subscription).await;
    let (customer, a, b) = two_subscriptions(&h).await;

    let refused = h
        .services()
        .redeem_coupon_credit_code_at(h.tenant, customer, "LAUNCHSUB", at(9, 12))
        .await
        .expect_err("SUBSCRIPTION_REQUIRED");
    assert!(
        format!("{refused:?}").contains("Choose the subscription this code applies to."),
        "{refused:?}"
    );
    h.services()
        .redeem_coupon_credit_code_for_at(h.tenant, customer, "LAUNCHSUB", Some(b), at(9, 12))
        .await
        .expect("redeemed for B");
    let grants = h.grants(customer).await;
    let promo = grant(&grants, "COUPON", "PROMOTIONAL", 10_000);
    assert_eq!(promo.subscription_id, Some(b));
    let promo = promo.id;
    h.check().await;

    let a2 = h.renew(a, at(10, 1)).await.expect("INV-A2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let a2 = h.invoice(a2.id).await;
    assert_eq!(
        money(&a2),
        (112_000, 0, 10_000, 0, 102_000, 50_000, 52_000),
        "B's promotion never pays A: {:?}",
        lines(&a2)
    );
    h.check().await;

    h.sweep(customer, at(10, 7) + Halyard::hours(1)).await;
    assert_eq!(
        h.movements(promo).await,
        vec!["FUND 10000", "DRAW -9000", "EXPIRY -1000"]
    );
    let b1 = h.renew(b, at(10, 10)).await.expect("INV-B1");
    let b1 = h.invoice(b1.id).await;
    assert_eq!(
        money(&b1),
        (9_000, 0, 9_000, 0, 0, 0, 0),
        "{:?}",
        lines(&b1)
    );
    h.check().await;
}
