//! S8. Scale with a 14-day free trial from Sep 1 and LAUNCH100 applied at signup (OQ9): nothing
//! is billed or granted during the trial, the promo code is minted when the trial starts, the
//! first period runs from the trial's end, and trial usage is never billed nor settled.
use common_domain::actor::Actor;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::{
    CreateSubscriptionCoupon, CreateSubscriptionCoupons, PlanTrial, TrialPatch,
};
use meteroid_store::repositories::PlansInterface;
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use rust_decimal_macros::dec;

use super::kit::{Halyard, LITE, M, Tax, at, at_time, day, expiring, grant, money, summary};

#[tokio::test]
async fn s08_free_trial_then_paid_with_a_promo_code_at_signup() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    h.store()
        .patch_trial(
            Actor::System,
            TrialPatch {
                plan_version_id: h.scale,
                tenant_id: h.tenant,
                trial: Some(PlanTrial {
                    duration_days: 14,
                    trialing_plan_id: None,
                    trial_is_free: true,
                }),
            },
        )
        .await
        .expect("Scale offers a free trial");
    let launch = h
        .services()
        .create_coupon_at(
            Actor::System,
            CouponNew {
                code: "LAUNCH100".to_owned(),
                description: "Launch credit".to_owned(),
                tenant_id: h.tenant,
                discount: CouponDiscount::Credit {
                    currency: "USD".to_owned(),
                    amount: dec!(100),
                    expires_in_days: Some(25),
                    scope: Default::default(),
                },
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
            at(8, 1),
        )
        .await
        .expect("the catalog's promo code");

    let mut request = h.request(customer, h.scale, day(9, 1));
    request.subscription.trial_duration = Some(14);
    request.coupons = Some(CreateSubscriptionCoupons {
        coupons: vec![CreateSubscriptionCoupon {
            coupon_id: launch.id,
        }],
    });
    // A trial cannot be replayed from its start (`insert_subscription_started_at` refuses it):
    // the subscription is created as started on Sep 1, its trial ending in business time below.
    let sub = h
        .services()
        .insert_subscription(Actor::System, request, h.tenant)
        .await
        .expect("trial subscription")
        .id;
    assert!(
        h.invoices(sub)
            .await
            .iter()
            .all(|invoice| invoice.line_items.is_empty() && invoice.total == 0),
        "nothing is billed during the trial"
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        summary(&grants),
        vec!["COUPON PROMOTIONAL 10000 10000"],
        "no pack credit before the invoice that bills it; LAUNCH100 minted at the trial start"
    );
    // Minted when the subscription was created (not in business time, see above), so its 25
    // days run from then: the Sep 26 lapse is in FOLLOWUPS "Scenario tests".
    let promo = grant(&grants, "COUPON", "PROMOTIONAL", 10_000);
    assert_eq!(
        (promo.subscription_id, promo.expires_at),
        (None, Some(promo.effective_at + chrono::Duration::days(25))),
        "customer-wide, the grant instant + 25 days (OQ20)"
    );
    h.check().await;

    // Trial usage: lite 25M = 5000, never billed.
    h.tokens(customer, at_time(9, 5, 10, 0, 0), LITE, 25 * M);

    let first = h
        .renew(sub, at(9, 15))
        .await
        .expect("INV-1 at the trial end");
    assert_eq!(first.billing_period_start, day(9, 15));
    assert_eq!(money(&first), (50_000, 0, 0, 0, 50_000, 0, 50_000));
    assert!(
        first.line_items.iter().all(|line| line.metric_id.is_none()),
        "trial usage is never billed"
    );
    h.pay(first.id, at(9, 15)).await;
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    assert_eq!(
        (paid.expires_at, bonus.expires_at),
        (Some(at(10, 15)), Some(at(10, 15))),
        "the period runs from the trial's end"
    );
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(12, 14));
    assert_eq!(
        (welcome.amount, welcome.subscription_id),
        (5_000, Some(sub)),
        "Sep 15 + 90 days"
    );
    h.check().await;

    let grants = h.grants(customer).await;
    assert_eq!(
        grants.iter().filter(|g| g.source_kind == "COUPON").count(),
        1,
        "minted once"
    );
    assert_eq!(
        h.movements(grant(&grants, "COUPON", "PROMOTIONAL", 10_000).id)
            .await,
        vec!["FUND 10000"],
        "the trial's usage is never settled"
    );
    h.check().await;
}
