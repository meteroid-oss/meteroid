//! S2. Scale → Studio from the portal on Sep 16: a downgrade, so it waits for Oct 1. The pack
//! settles September's usage at its expiry, its unused paid credit is breakage, and the renewal
//! bills Scale's usage in arrears with Studio's October fee.
use common_domain::actor::Actor;

use super::kit::{Halyard, LITE, M, MAX, STANDARD, Tax, at, at_time, day, grant, lines, money};
use meteroid_store::repositories::CreditNoteInterface;

#[tokio::test]
async fn s02_scale_to_studio_scheduled_at_period_end() {
    let h = Halyard::new().await;
    let customer = switched_at_renewal(&h).await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!((rev.billed, rev.breakage), (54_900, 10_000));
    }
}

/// S2's revenue as of Oct 1: recognized 40000 + breakage 10000; deferred 4900, October's fee.
#[tokio::test]
async fn s02_revenue_defers_the_fee_the_cash_paid() {
    let h = Halyard::new().await;
    let customer = switched_at_renewal(&h).await;
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (54_900, 50_000, 4_900, 10_000)
        );
    }
}

/// S2's paid pack lapses at the Oct 1 switch: its breakage belongs to Scale, the plan it was
/// bought under, not to Studio, the plan the subscription is on now.
#[tokio::test]
async fn s02_breakage_belongs_to_the_plan_it_lapsed_under() {
    let h = Halyard::new().await;
    let customer = switched_at_renewal(&h).await;
    let Some(rows) = h.revenue_rows(customer).await else {
        return;
    };
    let scale = rows
        .iter()
        .filter(|row| row.invoice_id.is_some() && row.grant_id.is_none())
        .min_by_key(|row| row.billed_at)
        .and_then(|row| row.plan_id)
        .expect("INV-1 bills Scale");
    let breakage: Vec<_> = rows
        .iter()
        .filter(|row| row.charge_type == "breakage")
        .collect();
    assert_eq!(breakage.len(), 1, "{breakage:?}");
    assert_eq!(breakage[0].recognized_subtotal_cents, 10_000);
    assert_eq!(breakage[0].plan_id, Some(scale), "Scale's pack, not Studio");
}

/// S2's timeline to the Oct 1 renewal, asserting its ledger and invoice figures.
async fn switched_at_renewal(h: &Halyard) -> common_domain::ids::CustomerId {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    let first = first.expect("INV-1");
    h.pay(first.id, at(9, 1)).await;

    h.tokens(customer, at_time(9, 2, 10, 0, 0), STANDARD, 25 * M);
    h.tokens(customer, at_time(9, 8, 10, 0, 0), LITE, 10 * M);
    h.tokens(customer, at_time(9, 15, 10, 0, 0), MAX, M / 2);

    let scheduled = h
        .services()
        .schedule_plan_change(Actor::System, sub, h.tenant, h.studio, vec![])
        .await
        .expect("the portal's change");
    assert_eq!(
        scheduled.scheduled_time.date(),
        day(10, 1),
        "a downgrade waits for the period end"
    );
    assert!(
        h.store()
            .list_credit_notes_by_invoice_id(h.tenant, first.id)
            .await
            .expect("notes")
            .is_empty()
    );
    h.check().await;

    // Sep 16-30: standard 8M (9600) + lite 2M (400).
    h.tokens(customer, at_time(9, 20, 10, 0, 0), STANDARD, 8 * M);
    h.tokens(customer, at_time(9, 30, 23, 0, 0), LITE, 2 * M);

    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (49_900, 0, 5_000, 0, 44_900, 40_000, 4_900),
        "Scale usage 45000 and Studio's October fee: {:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        h.movements(paid.id).await,
        vec!["FUND 50000", "DRAW -40000", "EXPIRY -10000"]
    );
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000).id)
            .await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    let welcome = grants
        .iter()
        .find(|g| g.source_kind == "INCLUDED_CREDIT" && g.expires_at == Some(at(11, 30)))
        .expect("the welcome credit");
    assert_eq!(welcome.remaining, 5_000, "not lapsing: untouched");
    let units = grants
        .iter()
        .find(|g| g.source_kind == "INCLUDED_CREDIT" && g.expires_at == Some(at(11, 1)))
        .expect("Studio's October credits");
    assert_eq!((units.amount, units.remaining), (5_000, 5_000));
    assert_eq!(h.denomination(units).await, "HALYARD_CREDITS");
    h.check().await;
    customer
}
