//! S16. A New York customer (8.875 %): promotional credit comes off before tax, paid credit pays
//! the taxed total at face; a discounted paid grant splits into paid and bonus. Revenue per OQ13:
//! billed net of all tax, including the tax paid with credit; deferred = paid credit left at cost.
use chrono::Duration;
use common_domain::actor::Actor;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::credit::GrantClassification;
use meteroid_store::domain::{CreateSubscriptionCoupon, CreateSubscriptionCoupons};
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use rust_decimal_macros::dec;

use super::kit::{
    Halyard, LITE, M, MAX, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};
use super::s13_top_up_void_note_refund::{add_credit, top_up};

/// `WELCOME80`: 80.00 of promotional credit, customer-wide, never expiring.
async fn welcome80(h: &Halyard) -> common_domain::ids::CouponId {
    h.services()
        .create_coupon_at(
            Actor::System,
            CouponNew {
                code: "WELCOME80".to_owned(),
                description: "Welcome credit".to_owned(),
                tenant_id: h.tenant,
                discount: CouponDiscount::Credit {
                    currency: "USD".to_owned(),
                    amount: dec!(80),
                    expires_in_days: None,
                    scope: Default::default(),
                },
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: false,
                plan_ids: vec![],
            },
            at(8, 31),
        )
        .await
        .expect("WELCOME80")
        .id
}

#[tokio::test]
async fn s16_a_promotional_credit_before_tax_then_paid_credit_at_face() {
    let h = Halyard::new().await;
    let customer = h.customer("NY", Tax::NewYork).await;
    let coupon = welcome80(&h).await;
    let mut request = h.request(customer, h.payg, day(9, 1));
    request.coupons = Some(CreateSubscriptionCoupons {
        coupons: vec![CreateSubscriptionCoupon { coupon_id: coupon }],
    });
    let (sub, _) = h.start(request).await;
    top_up(&h, customer, 50_000, at(9, 1)).await;
    let grants = h.grants(customer).await;
    let promo = grant(&grants, "COUPON", "PROMOTIONAL", 8_000);
    assert_eq!((promo.expires_at, promo.subscription_id), (None, None));
    h.check().await;

    // Sep: standard 25M (30000) + lite 50M (10000) = 40000.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 25 * M);
    h.tokens(customer, at_time(9, 20, 10, 0, 0), LITE, 50 * M);
    let invoice = h.renew(sub, at(10, 1)).await.expect("INV-1");
    assert_eq!(
        money(&invoice),
        (40_000, 0, 8_000, 2_840, 34_840, 34_840, 0),
        "32000 × 0.08875 = 2840 (3550 if paid credit went first): {:?}",
        lines(&invoice)
    );
    let grants = h.grants(customer).await;
    assert_eq!(grant(&grants, "COUPON", "PROMOTIONAL", 8_000).remaining, 0);
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000).remaining,
        15_160
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (47_160, 32_000, 15_160),
            "the 2840 of tax paid with the top-up is not revenue; deferred = top-up left at cost"
        );
    }
}

#[tokio::test]
async fn s16_b_pack_settlement_freezes_the_tax_its_paid_credit_pays() {
    let h = Halyard::new().await;
    let customer = h.customer("NY", Tax::NewYork).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    let first = first.expect("INV-1");
    assert_eq!(
        money(&first),
        (50_000, 0, 0, 0, 50_000, 0, 50_000),
        "a currency pack is untaxed"
    );
    h.pay(first.id, at(9, 1)).await;

    // Sep: standard 35M (42000) + max 0.5M (3000) = 45000.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 35 * M);
    h.tokens(customer, at_time(9, 20, 10, 0, 0), MAX, M / 2);
    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Duration::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (95_000, 0, 5_000, 3_550, 93_550, 43_550, 50_000),
        "October's pack untaxed; the usage taxed on 40000 after the bonus: {:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    let september_paid = grants
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PAID"
                && g.expires_at == Some(at(10, 1))
        })
        .expect("September's paid pack");
    assert_eq!(
        h.movements(september_paid.id).await,
        vec!["FUND 50000", "DRAW -43550", "EXPIRY -6450"],
        "usage 40000 and its 3550 of tax, frozen with the settlement"
    );
    let september_bonus = grants
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PROMOTIONAL"
                && g.expires_at == Some(at(10, 1))
        })
        .expect("September's bonus");
    assert_eq!(
        h.movements(september_bonus.id).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        5_000,
        "the welcome credit is not lapsing: untouched"
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (96_450, 46_450, 50_000, 6_450),
            "billed 50000 + 50000 − 3550; recognized (43550 − 3550) + 6450"
        );
    }
}

#[tokio::test]
async fn s16_c_discounted_paid_credit_splits_into_paid_and_bonus() {
    let h = Halyard::new().await;
    let customer = h.customer("NY", Tax::NewYork).await;
    let (sub, _) = h.subscribe(customer, h.payg, day(9, 1)).await;
    add_credit(
        &h,
        customer,
        10_000,
        GrantClassification::Paid,
        Some(9_000),
        at(9, 1),
    )
    .await;
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "MANUAL", "PAID", 9_000);
    let bonus = grant(&grants, "MANUAL", "PROMOTIONAL", 1_000);
    assert_eq!(paid.cost_minor, Some(9_000));
    assert_eq!(bonus.cost_minor.unwrap_or(0), 0);
    assert_eq!(bonus.reason, "Bonus credit");
    assert_eq!(bonus.expires_at, paid.expires_at);
    h.check().await;

    // standard 7.5M = 9000.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 15 * M / 2);
    let invoice = h.renew(sub, at(10, 1)).await.expect("INV");
    assert_eq!(
        money(&invoice),
        (9_000, 0, 1_000, 710, 8_710, 8_710, 0),
        "the bonus before tax, 8000 × 0.08875 = 710: {:?}",
        lines(&invoice)
    );
    let grants = h.grants(customer).await;
    assert_eq!(grant(&grants, "MANUAL", "PAID", 9_000).remaining, 290);
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (8_290, 8_000, 290),
            "billed 9000 − 710; deferred = the paid credit left"
        );
    }
}
