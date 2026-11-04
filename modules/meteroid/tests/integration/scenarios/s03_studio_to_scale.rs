//! S3. Studio → Scale: the Studio fee drops to 0 and the unit pack is removed, so it is a
//! downgrade (X5) and waits for Oct 1. Studio's credits settle September and roll over (OQ5) into
//! a rollover Scale cannot use; Scale's welcome credit is granted on its first invoice (OQ6).
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, SubscriptionId};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::CreditType;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};

/// Studio from Sep 1, its first invoice (fee 4900) paid at once: 5000 credits to Oct 1.
async fn on_studio(h: &Halyard) -> (CustomerId, SubscriptionId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.studio, day(9, 1)).await;
    let first = first.expect("INV-0");
    assert_eq!(money(&first), (4_900, 0, 0, 0, 4_900, 0, 4_900));
    h.pay(first.id, at(9, 1)).await;
    let grants = h.grants(customer).await;
    let units = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
    assert_eq!(h.denomination(units).await, "HALYARD_CREDITS");
    assert_eq!(
        (
            units.amount,
            units.classification.as_str(),
            units.subscription_id
        ),
        (5_000, "PROMOTIONAL", Some(sub))
    );
    h.check().await;
    (customer, sub)
}

#[tokio::test]
async fn s03_studio_to_scale_waits_for_the_period_end() {
    let h = Halyard::new().await;
    let (customer, sub) = on_studio(&h).await;
    // 4200 credits: standard 3.5M.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 7 * M / 2);

    let scheduled = h
        .services()
        .schedule_plan_change(Actor::System, sub, h.tenant, h.scale, vec![])
        .await
        .expect("the portal's change");
    assert_eq!(scheduled.scheduled_time.date(), day(10, 1), "a downgrade");
    h.check().await;

    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-1");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (50_000, 0, 0, 0, 50_000, 0, 50_000),
        "Studio usage covered by its credits, Scale's October pack, the welcome credit: {:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    let september = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
    assert_eq!(
        h.movements(september.id).await,
        vec!["FUND 5000", "DRAW -4200", "ROLLOVER -800"]
    );
    let rolled = grant(&grants, "ROLLOVER", "PROMOTIONAL", 800);
    assert_eq!(
        (
            rolled.subscription_id,
            rolled.priority,
            rolled.expires_at,
            rolled.remaining
        ),
        (Some(sub), september.priority - 1, Some(at(11, 1)), 800),
        "OQ5: the rollover happens though Scale does not price the unit"
    );
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(12, 30));
    assert_eq!(
        (welcome.amount, welcome.subscription_id),
        (5_000, Some(sub)),
        "OQ6: the first invoice of the subscription billing Scale's welcome product, Oct 1 + 90 d"
    );
    h.check().await;

    h.pay(renewal.id, at(10, 1)).await;
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    assert_eq!(
        (paid.expires_at, bonus.expires_at),
        (Some(at(11, 1)), Some(at(11, 1)))
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (54_900, 4_900, 50_000)
        );
    }

    // R1 is unusable on Scale and lapses at no cost on Nov 1.
    h.sweep(customer, at(11, 1) + Halyard::hours(1)).await;
    assert_eq!(
        h.movements(rolled.id).await,
        vec!["FUND 800", "EXPIRY -800"]
    );
    h.check().await;
}

/// S3-forced: an admin moves Studio to Scale on Sep 16. Studio's fee for the 15 days left comes
/// back to balance; its credit pays the usage to the change and the rest is removed (Settlement
/// fix round: an EXPIRY "Removed: plan change" after the grace); Scale's pack is prorated.
#[tokio::test]
async fn s03_forced_prorates_scale_s_pack_and_credits_studio_s_fee() {
    let h = Halyard::new().await;
    let (customer, sub) = on_studio(&h).await;
    // 2000 credits: lite 10M.
    h.tokens(customer, at_time(9, 5, 10, 0, 0), LITE, 10 * M);

    h.services()
        .apply_plan_change_immediate_at(Actor::System, sub, h.tenant, h.scale, vec![], day(9, 16))
        .await
        .expect("an admin forces the change");
    h.sweep(customer, at(9, 16) + Halyard::hours(1)).await;

    let grants = h.grants(customer).await;
    let units = expiring(&grants, "INCLUDED_CREDIT", at(10, 1));
    assert_eq!(
        h.movements_with_reasons(units.id).await,
        vec![
            "FUND 5000",
            "DRAW -2000",
            "EXPIRY -3000 Removed: plan change"
        ]
    );
    let invoices = h.invoices(sub).await;
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, invoices[0].id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1, "OQ1: a note on INV-0");
    assert_eq!(notes[0].credit_type, CreditType::CreditToBalance);
    assert_eq!(
        (notes[0].subtotal, notes[0].total),
        (-2_450, -2_450),
        "4900 × 15/30"
    );
    let note_credit = grant(&grants, "CREDIT_NOTE", "PAID", 2_450);
    assert_eq!(note_credit.subscription_id, None);

    let adjustment = &invoices[1];
    assert_eq!(
        money(adjustment),
        (25_000, 0, 0, 0, 25_000, 0, 25_000),
        "the prorated pack; the note's credit cannot pay a credit purchase (R4): {:?}",
        lines(adjustment)
    );
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(12, 15));
    assert_eq!(
        (welcome.amount, welcome.subscription_id),
        (5_000, Some(sub)),
        "Sep 16 + 90 d"
    );
    h.check().await;

    h.pay(adjustment.id, at(9, 16)).await;
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 2_500);
    assert_eq!(
        (
            paid.subscription_id,
            paid.expires_at,
            bonus.expires_at,
            paid.cost_minor
        ),
        (Some(sub), Some(at(10, 1)), Some(at(10, 1)), Some(25_000)),
        "27500 of credit for 25000: Paid 25000 + Bonus 2500"
    );
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 2_450).remaining,
        2_450,
        "kept for later invoices"
    );
    h.check().await;
}
