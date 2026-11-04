//! S17. Studio's included credits at period boundaries: periods are half-open, a rollover child
//! burns before the fresh credit lapsing with it (OQ4), it never rolls again, and usage reaching
//! the metering store after the settlement grace is paid only by credit effective in its own
//! period, the rest billed beyond credit (OQ15; founder 2026-10-20, Orb). The rollover reaches the
//! next invoice whether the sweep or that invoice runs first.
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CustomerId, SubscriptionId};
use meteroid_store::domain::credit::GrantClassification;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money, summary,
};
use super::s13_top_up_void_note_refund::top_up;

/// Credits added by hand at `at`: customer-wide, any charge, no expiry; returns the grant.
async fn add_credits(
    h: &Halyard,
    customer: CustomerId,
    amount: i64,
    at: DateTime<Utc>,
) -> common_domain::ids::CreditGrantId {
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "HALYARD_CREDITS")
        .await
        .expect("the credits wallet")
        .id;
    h.store()
        .fund_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet,
                amount_minor: amount,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Scenario credits".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            at,
        )
        .await
        .expect("credits added")
        .grant_id
}

/// Studio from Aug 1 with 3800 credits of August usage, the last 200 at Aug 31 23:59:59, and a
/// lite event at Sep 1 00:00:00 that belongs to September. Returns once Sep 1 is billed and paid,
/// before any sweep: A1 still holds its 1200 credits left.
async fn studio_billed_through_august(h: &Halyard, name: &str) -> (CustomerId, SubscriptionId) {
    let customer = h.customer(name, Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.studio, day(8, 1)).await;
    let first = first.expect("INV-0");
    assert_eq!(money(&first), (4_900, 0, 0, 0, 4_900, 0, 4_900));
    h.pay(first.id, at(8, 1)).await;
    let grants = h.grants(customer).await;
    let a1 = expiring(&grants, "INCLUDED_CREDIT", at(9, 1));
    assert_eq!(h.denomination(a1).await, "HALYARD_CREDITS");
    assert_eq!((a1.amount, a1.priority), (5_000, 100));

    // August: standard 3M (3600 credits) + lite 1M (200 credits) at its last second.
    h.tokens(customer, at_time(8, 10, 10, 0, 0), STANDARD, 3 * M);
    h.tokens(customer, at_time(8, 31, 23, 59, 59), LITE, M);
    h.tokens(customer, at(9, 1), LITE, M);

    let september = h.renew(sub, at(9, 1)).await.expect("INV-1");
    assert_eq!(
        money(&september),
        (4_900, 0, 0, 0, 4_900, 0, 4_900),
        "August's 3800 credits covered, September's fee: {:?}",
        lines(&september)
    );
    h.pay(september.id, at(9, 1)).await;
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(9, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -3800"]
    );
    (customer, sub)
}

/// `studio_billed_through_august`, then the sweep an hour after Sep 1 rolls A1 over.
async fn studio_through_august(h: &Halyard) -> (CustomerId, SubscriptionId) {
    let (customer, sub) = studio_billed_through_august(h, "Untaxed").await;
    h.sweep(customer, at(9, 1) + Halyard::hours(1)).await;

    let grants = h.grants(customer).await;
    let a1 = expiring(&grants, "INCLUDED_CREDIT", at(9, 1));
    assert_eq!(
        h.movements(a1.id).await,
        vec!["FUND 5000", "DRAW -3800", "ROLLOVER -1000", "EXPIRY -200"],
        "the Sep 1 00:00:00 event is September's: A1 cannot pay it"
    );
    let r1 = expiring(&grants, "ROLLOVER", at(10, 1));
    assert_eq!(
        (
            r1.amount,
            r1.remaining,
            r1.classification.as_str(),
            r1.subscription_id,
            r1.priority,
            r1.effective_at
        ),
        (1_000, 1_000, "PROMOTIONAL", Some(sub), 99, at(9, 1)),
        "capped at 1000, a month from the parent's lapse, burning before fresh credit"
    );
    assert!(r1.rollover.is_none(), "a rollover never rolls again");
    let s2 = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
    assert_eq!((s2.amount, s2.remaining, s2.priority), (5_000, 5_000, 100));
    h.check().await;
    if let Some(rev) = h.revenue(customer, day(9, 1)).await {
        assert_eq!(rev.breakage, 0, "included credit lapses at cost 0");
    }
    (customer, sub)
}

/// Sep 2-30 usage on top of the Sep 1 00:00:00 event (200): `standard` and `lite` millions.
fn september_usage(h: &Halyard, customer: CustomerId, standard: u64, lite: u64) {
    if standard > 0 {
        h.tokens(customer, at_time(9, 12, 10, 0, 0), STANDARD, standard);
    }
    if lite > 0 {
        h.tokens(customer, at_time(9, 30, 23, 59, 59), LITE, lite);
    }
}

/// 5600 credits: the Sep 1 00:00:00 event (200), standard 4M (4800), lite 2M (400) and lite 1M
/// (200) at Sep 30 23:59:59.
fn september_5600(h: &Halyard, customer: CustomerId) {
    september_usage(h, customer, 4 * M, M);
    h.tokens(customer, at_time(9, 20, 10, 0, 0), LITE, 2 * M);
}

#[tokio::test]
async fn s17_rollover_burns_first_and_rolls_once() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    september_5600(&h, customer);

    let october = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (4_900, 0, 0, 0, 4_900, 0, 4_900),
        "September's 5600 credits covered: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    let r1 = expiring(&grants, "ROLLOVER", at(10, 1));
    assert_eq!(
        h.movements(r1.id).await,
        vec!["FUND 1000", "DRAW -1000"],
        "R1 ranks first among the credit lapsing Oct 1 (priority 99, OQ4)"
    );
    let s2 = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
    assert_eq!(
        h.movements(s2.id).await,
        vec!["FUND 5000", "DRAW -4600", "ROLLOVER -400"]
    );
    let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
    assert_eq!(
        (r2.amount, r2.remaining, r2.priority, r2.subscription_id),
        (400, 400, 99, Some(sub))
    );
    assert_eq!(
        grants
            .iter()
            .filter(|g| g.source_kind == "ROLLOVER")
            .count(),
        2,
        "R1 never rolls: {:?}",
        summary(&grants)
    );
    let s3 = expiring(&grants, "INCLUDED_CREDIT", at(11, 1));
    assert_eq!((s3.amount, s3.remaining), (5_000, 5_000));
    h.check().await;
}

/// The sweep rolls A1 over before October's invoice, or only after it (periods invoiced in
/// catch-up, a lagging sweep): either way the invoice rolls what the sweep has not and spends R1
/// before S2, A1 rolls once, and the sweep after it finds nothing of A1 left. The credits are a
/// unit's: the invoice draws them through the unit wallet's cover (`draw_unit_cover`).
#[tokio::test]
async fn s17_rollover_reaches_the_next_invoice_whichever_runs_first() {
    let h = Halyard::new().await;
    for (name, swept_first) in [("Swept first", true), ("Invoiced first", false)] {
        let (customer, sub) = studio_billed_through_august(&h, name).await;
        if swept_first {
            h.sweep(customer, at(9, 1) + Halyard::hours(1)).await;
        }
        september_5600(&h, customer);

        let october = h.renew(sub, at(10, 1)).await.expect("INV-2");
        assert_eq!(
            money(&october),
            (4_900, 0, 0, 0, 4_900, 0, 4_900),
            "{name}: September's 5600 credits covered: {:?}",
            lines(&october)
        );
        let grants = h.grants(customer).await;
        let a1 = expiring(&grants, "INCLUDED_CREDIT", at(9, 1));
        let a1_movements = vec!["FUND 5000", "DRAW -3800", "ROLLOVER -1000", "EXPIRY -200"];
        assert_eq!(h.movements(a1.id).await, a1_movements, "{name}");
        let r1 = expiring(&grants, "ROLLOVER", at(10, 1));
        assert_eq!(
            (r1.amount, r1.priority, r1.subscription_id, r1.effective_at),
            (1_000, 99, Some(sub), at(9, 1)),
            "{name}"
        );
        assert_eq!(
            h.movements(r1.id).await,
            vec!["FUND 1000", "DRAW -1000"],
            "{name}: R1 burns before S2"
        );
        let s2 = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
        assert_eq!(
            h.movements(s2.id).await,
            vec!["FUND 5000", "DRAW -4600"],
            "{name}: S2 rolls with the sweep, its rollover cannot pay September"
        );

        h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
        let grants = h.grants(customer).await;
        assert_eq!(h.movements(a1.id).await, a1_movements, "{name}");
        assert_eq!(
            grants
                .iter()
                .filter(|g| g.source_kind == "ROLLOVER" && g.source_id == a1.id.as_uuid())
                .count(),
            1,
            "{name}: A1 rolls once: {:?}",
            summary(&grants)
        );
        assert_eq!(
            h.movements(s2.id).await,
            vec!["FUND 5000", "DRAW -4600", "ROLLOVER -400"],
            "{name}"
        );
        let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
        assert_eq!((r2.amount, r2.remaining), (400, 400), "{name}");
        h.check().await;
    }
}

/// A lite 1M (200 credits) dated Sep 30 23:59:59 reaches the metering store at Oct 1 00:00:30,
/// inside the 30-minute settlement grace: September's credit pays it before the roll. The period
/// invoice is issued at 01:00, after the grace, so it can still bill that usage.
#[tokio::test]
async fn s17_late_event_inside_the_grace_is_paid_before_the_roll() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    september_5600(&h, customer);
    h.tokens(customer, at_time(9, 30, 23, 59, 59), LITE, M);

    h.sweep(customer, at(10, 1) + chrono::Duration::minutes(30))
        .await;
    let october = h
        .renew(sub, at(10, 1) + Halyard::hours(1))
        .await
        .expect("INV-2");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (4_900, 0, 0, 0, 4_900, 0, 4_900),
        "5800 credits, all covered: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(10, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -4800", "ROLLOVER -200"]
    );
    let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
    assert_eq!((r2.amount, r2.remaining), (200, 200));
    h.check().await;
}

/// The same late event reaching the metering store after the grace: September's credit has
/// rolled, so the invoice pays it with the credit valid in September that is left, its rollover
/// (founder 2026-10-20: late usage is paid only by credit valid in its own period).
#[tokio::test]
async fn s17_late_event_after_the_grace_is_paid_by_septembers_rollover() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    september_5600(&h, customer);

    h.sweep(customer, at(10, 1) + chrono::Duration::minutes(30))
        .await;
    h.tokens(customer, at_time(9, 30, 23, 59, 59), LITE, M);
    let october = h
        .renew(sub, at(10, 1) + Halyard::hours(1))
        .await
        .expect("INV-2");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (4_900, 0, 0, 0, 4_900, 0, 4_900),
        "the 200 late credits are covered, October's fee due: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(10, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -4600", "ROLLOVER -400"]
    );
    let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
    assert_eq!(h.movements(r2.id).await, vec!["FUND 400", "DRAW -200"]);
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 1)).remaining,
        5_000
    );
    h.check().await;
}

/// 1000 late credits after the grace: September's rollover pays its 400, and the 600 left are
/// billed beyond credit ($6.00). Credits added on Oct 1 at 00:45, before the invoice, are not
/// effective in September: they never pay its usage and stay whole.
#[tokio::test]
async fn s17_late_usage_beyond_the_rollover_is_billed_not_paid_by_october_credit() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    september_5600(&h, customer);

    h.sweep(customer, at(10, 1) + chrono::Duration::minutes(30))
        .await;
    let october_credit = add_credits(
        &h,
        customer,
        2_000,
        at(10, 1) + chrono::Duration::minutes(45),
    )
    .await;
    h.tokens(customer, at_time(9, 30, 23, 59, 59), LITE, 5 * M);
    let october = h
        .renew(sub, at(10, 1) + Halyard::hours(1))
        .await
        .expect("INV-2");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (5_500, 0, 0, 0, 5_500, 0, 5_500),
        "October's fee and 600 late credits beyond credit: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(10, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -4600", "ROLLOVER -400"]
    );
    let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
    assert_eq!(h.movements(r2.id).await, vec!["FUND 400", "DRAW -400"]);
    assert_eq!(
        h.movements(october_credit).await,
        vec!["FUND 2000"],
        "added after September: it pays October onwards only"
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 1)).remaining,
        5_000,
        "October's included credit never pays September"
    );
    h.check().await;
}

/// Control: credits added on Sep 20 were effective in September, so they pay the 600 late
/// credits September's rollover leaves.
#[tokio::test]
async fn s17_late_usage_is_paid_by_credit_added_during_its_period() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    let added = add_credits(&h, customer, 2_000, at(9, 20)).await;
    september_5600(&h, customer);

    h.sweep(customer, at(10, 1) + chrono::Duration::minutes(30))
        .await;
    h.tokens(customer, at_time(9, 30, 23, 59, 59), LITE, 5 * M);
    let october = h
        .renew(sub, at(10, 1) + Halyard::hours(1))
        .await
        .expect("INV-2");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (4_900, 0, 0, 0, 4_900, 0, 4_900),
        "the 1000 late credits are covered: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
    assert_eq!(h.movements(r2.id).await, vec!["FUND 400", "DRAW -400"]);
    assert_eq!(h.movements(added).await, vec!["FUND 2000", "DRAW -600"]);
    h.check().await;
}

/// Money credit, same rule: Scale's September usage (72000 on time, 2000 late) is settled by
/// September's pack, its bonus and the welcome credit (60000) at the expiry sweep. A $200 top-up
/// bought on Oct 1 at 00:45, before the invoice, pays none of the 14000 left: it is billed in
/// cash with October's pack, and the top-up stays whole for October.
#[tokio::test]
async fn s17_a_top_up_bought_after_the_period_pays_none_of_its_usage() {
    let h = Halyard::new().await;
    let customer = h.customer("Scale late", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    h.pay(first.expect("INV-1").id, at(9, 1)).await;
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 60 * M);

    h.sweep(customer, at(10, 1) + chrono::Duration::minutes(30))
        .await;
    let purchase = top_up(
        &h,
        customer,
        20_000,
        at(10, 1) + chrono::Duration::minutes(45),
    )
    .await;
    h.tokens(customer, at_time(9, 30, 23, 59, 59), LITE, 10 * M);
    let october = h
        .renew(sub, at(10, 1) + Halyard::hours(1))
        .await
        .expect("INV-2");
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (124_000, 0, 10_000, 0, 114_000, 50_000, 64_000),
        "October's pack 50000 and September's 74000 of usage, 60000 of it settled: {:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    let bought = grants
        .iter()
        .find(|g| g.invoice_id == Some(purchase.id))
        .unwrap_or_else(|| panic!("the top-up's grant among {:?}", summary(&grants)));
    assert_eq!(
        h.movements(bought.id).await,
        vec!["FUND 20000"],
        "bought after September: it never pays September's usage"
    );
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000).remaining,
        0
    );
    h.check().await;
}

/// Variant: 6200 credits in September spend R1 and S2 whole; 200 credits are billed beyond credit
/// and nothing rolls over.
#[tokio::test]
async fn s17_overspent_september_rolls_nothing() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    // 200 + standard 5M (6000) = 6200.
    september_usage(&h, customer, 5 * M, 0);

    let october = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (5_100, 0, 0, 0, 5_100, 0, 5_100),
        "{:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "ROLLOVER", at(10, 1)).id)
            .await,
        vec!["FUND 1000", "DRAW -1000"]
    );
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(10, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    assert!(
        grants
            .iter()
            .all(|g| g.source_kind != "ROLLOVER" || g.expires_at != Some(at(11, 1))),
        "nothing rolls: {:?}",
        summary(&grants)
    );
    h.check().await;
}

/// Variant: 4500 credits in September: R1 pays 1000, S2 3500; S2's 1500 left rolls up to the cap
/// (1000) and the other 500 lapses.
#[tokio::test]
async fn s17_capped_roll_lapses_the_rest() {
    let h = Halyard::new().await;
    let (customer, sub) = studio_through_august(&h).await;
    // 200 + standard 3.5M (4200) + lite 0.5M (100) = 4500.
    september_usage(&h, customer, 3 * M + M / 2, M / 2);

    let october = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let october = h.invoice(october.id).await;
    assert_eq!(
        money(&october),
        (4_900, 0, 0, 0, 4_900, 0, 4_900),
        "{:?}",
        lines(&october)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(expiring(&grants, "INCLUDED_CREDIT", at(10, 1)).id)
            .await,
        vec!["FUND 5000", "DRAW -3500", "ROLLOVER -1000", "EXPIRY -500"]
    );
    let r2 = expiring(&grants, "ROLLOVER", at(11, 1));
    assert_eq!((r2.amount, r2.remaining), (1_000, 1_000));
    h.check().await;
}
