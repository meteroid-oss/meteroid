//! S1. Scale → Studio forced by an admin on Sep 16 (a downgrade, the pack is removed): the pack's
//! credit settles the usage to the change, its fee comes back on a capped credit note to balance,
//! Studio is billed and granted for the 15 days left, old-plan usage is billed on the renewal.
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, SubscriptionId};

use super::kit::{
    Halyard, LITE, M, MAX, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::CreditType;

/// Scale from Sep 1, its first invoice paid at once: P 50000, B 5000, W 5000.
async fn on_scale(h: &Halyard) -> (CustomerId, SubscriptionId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    let first = first.expect("the first invoice");
    assert_eq!(money(&first), (50_000, 0, 0, 0, 50_000, 0, 50_000));
    h.pay(first.id, at(9, 1)).await;
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        (
            paid.cost_minor,
            paid.subscription_id,
            paid.application.as_str(),
            paid.expires_at
        ),
        (Some(50_000), Some(sub), "USAGE", Some(at(10, 1)))
    );
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000).expires_at,
        Some(at(10, 1))
    );
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(11, 30));
    assert_eq!(welcome.classification, "PROMOTIONAL");
    assert_eq!(
        (welcome.subscription_id, welcome.expires_at),
        (Some(sub), Some(at(11, 30))),
        "Sep 1 + 90 days"
    );
    h.check().await;
    (customer, sub)
}

async fn change_to_studio(h: &Halyard, sub: SubscriptionId) {
    h.services()
        .apply_plan_change_immediate_at(Actor::System, sub, h.tenant, h.studio, vec![], day(9, 16))
        .await
        .expect("an admin forces the downgrade");
}

#[tokio::test]
async fn s01_scale_to_studio_forced_mid_period() {
    let h = Halyard::new().await;
    let customer = forced_to_studio_then_renewed(&h).await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.breakage),
            (50_000, 0),
            "50000 − 20000 (CN-1) + 20000 (G-cn, prepaid); P's voided 20000 is credited back"
        );
    }
}

/// The revenue of S1 as of Oct 1: recognized 30000 (usage via P) + 2450 (the Sep 16-Oct 1 fee via
/// G-cn); deferred 17550 = G-cn 12650 + October's fee 4900.
#[tokio::test]
async fn s01_revenue_follows_the_lines_credit_paid() {
    let h = Halyard::new().await;
    let customer = forced_to_studio_then_renewed(&h).await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (50_000, 32_450, 17_550, 0),
            "deferred = G-cn 12650 + October's fee 4900"
        );
    }
}

/// S1's timeline to the Oct 1 renewal, asserting every ledger and invoice figure on the way.
async fn forced_to_studio_then_renewed(h: &Halyard) -> CustomerId {
    let (customer, sub) = on_scale(h).await;
    if let Some(rev) = h.revenue(customer, day(9, 1)).await {
        assert_eq!((rev.billed, rev.recognized), (50_000, 0));
    }

    // Sep 2-15: standard 25M (30000) + lite 10M (2000) + max 0.5M (3000) = 35000.
    h.tokens(customer, at_time(9, 2, 10, 0, 0), STANDARD, 25 * M);
    h.tokens(customer, at_time(9, 8, 10, 0, 0), LITE, 10 * M);
    h.tokens(customer, at_time(9, 15, 10, 0, 0), MAX, M / 2);

    change_to_studio(h, sub).await;
    h.sweep(customer, at(9, 16) + Halyard::hours(1)).await;

    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    assert_eq!(
        h.movements(paid.id).await,
        vec!["FUND 50000", "DRAW -30000", "REVERSAL -20000"],
        "settled 30000, the capped fee credit of 20000 reversed: min(25000, 50000 × 20000/50000)"
    );
    assert_eq!(h.movements(bonus.id).await, vec!["FUND 5000", "DRAW -5000"]);
    let invoices = h.invoices(sub).await;
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, invoices[0].id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1, "CN-1 on INV-1");
    assert_eq!(notes[0].credit_type, CreditType::CreditToBalance);
    assert_eq!((notes[0].subtotal, notes[0].total), (-20_000, -20_000));
    let note_credit = grant(&grants, "CREDIT_NOTE", "PAID", 20_000);
    assert_eq!(
        (note_credit.subscription_id, note_credit.expires_at),
        (None, None)
    );

    let adjustment = &invoices[1];
    assert_eq!(
        money(adjustment),
        (2_450, 0, 0, 0, 2_450, 2_450, 0),
        "Studio fee 4900 × 15/30 paid by the note's credit"
    );
    let units = grant(&grants, "INCLUDED_CREDIT", "PROMOTIONAL", 2_500);
    assert_eq!(h.denomination(units).await, "HALYARD_CREDITS");
    assert_eq!(
        (units.subscription_id, units.expires_at),
        (Some(sub), Some(at(10, 1)))
    );
    assert_eq!(note_credit.remaining, 17_550);
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        5_000
    );
    h.check().await;

    // Sep 16-30 on Studio: standard 2M (2400 credits) + lite 1M (200 credits) = 2600 credits.
    h.tokens(customer, at_time(9, 20, 10, 0, 0), STANDARD, 2 * M);
    h.tokens(customer, at_time(9, 25, 10, 0, 0), LITE, M);

    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-3");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (40_000, 0, 5_100, 0, 34_900, 34_900, 0),
        "old-plan usage in arrears 35000, 100 credits beyond Studio's 2500, October's fee: {:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 20_000).remaining,
        12_650
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        4_900,
        "the welcome credit pays the 100 beyond Studio's credits"
    );
    assert_eq!(
        h.movements(units.id).await,
        vec!["FUND 2500", "DRAW -2500"],
        "nothing left to roll"
    );
    h.check().await;
    customer
}

/// S1-low: 10000 of usage to Sep 16. The fee credit is the time left (25000); the other 20000 of
/// unused paid credit is removed too and is breakage (OQ3).
#[tokio::test]
async fn s01_low_usage_books_the_unrefunded_rest_as_breakage() {
    let h = Halyard::new().await;
    let (customer, sub) = on_scale(&h).await;
    h.tokens(customer, at_time(9, 5, 10, 0, 0), STANDARD, 8 * M);
    h.tokens(customer, at_time(9, 6, 10, 0, 0), LITE, 2 * M);

    change_to_studio(&h, sub).await;
    h.sweep(customer, at(9, 16) + Halyard::hours(1)).await;

    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        h.movements_with_reasons(paid.id).await,
        vec![
            "FUND 50000",
            "DRAW -5000",
            "REVERSAL -25000 Removed: plan change",
            "EXPIRY -20000 Removed: plan change",
        ]
    );
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 25_000).subscription_id,
        None
    );
    h.check().await;
    if let Some(rev) = h.revenue(customer, day(9, 17)).await {
        assert_eq!(rev.breakage, 20_000, "the unrefunded paid credit, at cost");
    }
}

/// S1-burn: 60000 of usage to Sep 16 spends the pack whole: no fee credit; 5000 of Scale usage is
/// billed on the renewal and paid by the welcome credit.
#[tokio::test]
async fn s01_burn_leaves_no_fee_credit() {
    let h = Halyard::new().await;
    let (customer, sub) = on_scale(&h).await;
    h.tokens(customer, at_time(9, 5, 10, 0, 0), STANDARD, 50 * M);

    change_to_studio(&h, sub).await;
    h.sweep(customer, at(9, 16) + Halyard::hours(1)).await;
    let invoices = h.invoices(sub).await;
    assert!(
        h.store()
            .list_credit_notes_by_invoice_id(h.tenant, invoices[0].id)
            .await
            .expect("notes")
            .is_empty(),
        "fee credit = min(25000, 0)"
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000).id)
            .await,
        vec!["FUND 50000", "DRAW -50000"]
    );

    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-3");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    let grants = h.grants(customer).await;
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        0,
        "the 5000 of Scale usage beyond the pack is paid by the welcome credit: {:?}",
        lines(&renewal)
    );
    h.check().await;
}
