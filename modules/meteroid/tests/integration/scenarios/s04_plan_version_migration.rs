//! S4. Scale v1 → Scale v2, whose pack grants 60000 for the same 50000: a version migration.
//! By default it takes effect at the renewal (OQ7); an operator may force it at once, with the
//! mechanics of an immediate change (R15). The welcome credit is never granted again (R16).
use common_domain::actor::Actor;
use common_domain::ids::{CustomerId, PlanVersionId, SubscriptionId};
use meteroid_store::domain::enums::BillingPeriodEnum;
use meteroid_store::domain::price_components::{CreditExpiry, PriceEntry, PriceInput, ProductRef};
use meteroid_store::domain::prices::Pricing;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::PlansInterface;
use meteroid_store::repositories::credit_notes::CreditType;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;
use rust_decimal_macros::dec;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};

/// Scale v2: a copy of Scale whose pack grants $600 for $500.
async fn scale_v2(h: &Halyard) -> PlanVersionId {
    let draft = h
        .store()
        .copy_plan_version_to_draft(h.scale, h.tenant)
        .await
        .expect("a draft of Scale")
        .id;
    let pack = h
        .store()
        .list_price_components(draft, h.tenant)
        .await
        .expect("components")
        .into_iter()
        .find(|component| component.name == "Monthly credit pack")
        .expect("the pack");
    h.store()
        .delete_price_component(pack.id, h.tenant)
        .await
        .expect("drop the v1 pack");
    h.store()
        .create_price_component_from_ref(
            "Monthly credit pack".to_owned(),
            ProductRef::Existing(pack.product_id.expect("the pack's product")),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: dec!(500),
                    quantity: 1,
                    credit_amount: dec!(600),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    credit_unit_id: None,
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            })],
            draft,
            h.tenant,
        )
        .await
        .expect("the v2 pack");
    h.store()
        .publish_plan_version(Actor::System, draft, h.tenant)
        .await
        .expect("Scale v2 published")
        .id
}

/// Scale v1 from Sep 1, paid; 30000 of usage Sep 1-15.
async fn on_scale_v1(h: &Halyard) -> (CustomerId, SubscriptionId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    h.pay(first.expect("INV-1").id, at(9, 1)).await;
    h.tokens(customer, at_time(9, 5, 10, 0, 0), STANDARD, 25 * M);
    h.check().await;
    (customer, sub)
}

/// Sep 16-30: standard 15M (18000) + lite 10M (2000) = 20000.
fn second_half(h: &Halyard, customer: CustomerId) {
    h.tokens(customer, at_time(9, 20, 10, 0, 0), STANDARD, 15 * M);
    h.tokens(customer, at_time(9, 25, 10, 0, 0), LITE, 10 * M);
}

async fn assert_welcome_once(h: &Halyard, customer: CustomerId) {
    let grants = h.grants(customer).await;
    let included: Vec<_> = grants
        .iter()
        .filter(|g| g.source_kind == "INCLUDED_CREDIT")
        .collect();
    assert_eq!(
        included.len(),
        1,
        "R16: the welcome credit is not granted again"
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        5_000
    );
}

#[tokio::test]
async fn s04_version_migration_takes_effect_at_renewal() {
    let h = Halyard::new().await;
    let v2 = scale_v2(&h).await;
    let (customer, sub) = on_scale_v1(&h).await;

    let scheduled = h
        .services()
        .schedule_plan_change(Actor::System, sub, h.tenant, v2, vec![])
        .await
        .expect("the operator migrates the subscription");
    assert_eq!(
        scheduled.scheduled_time.date(),
        day(10, 1),
        "OQ7: at renewal"
    );
    second_half(&h, customer);

    let renewal = h.renew(sub, at(10, 1)).await.expect("the October invoice");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (100_000, 0, 5_000, 0, 95_000, 45_000, 50_000),
        "v2 pack 50000 and September's usage 50000: {:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        h.movements(paid.id).await,
        vec!["FUND 50000", "DRAW -45000", "EXPIRY -5000"]
    );
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000).id)
            .await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    h.check().await;

    h.pay(renewal.id, at(10, 1)).await;
    let grants = h.grants(customer).await;
    let october_paid = grants
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PAID"
                && g.expires_at == Some(at(11, 1))
        })
        .expect("October's paid pack credit");
    assert_eq!(october_paid.amount, 50_000);
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 10_000).expires_at,
        Some(at(11, 1)),
        "v2: 60000 for 50000 is Paid 50000 + Bonus 10000"
    );
    assert_welcome_once(&h, customer).await;
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (100_000, 50_000, 50_000, 5_000)
        );
    }
}

#[tokio::test]
async fn s04_forced_version_migration_prorates_both_packs() {
    let h = Halyard::new().await;
    let v2 = scale_v2(&h).await;
    let (customer, sub) = on_scale_v1(&h).await;

    h.services()
        .apply_plan_change_immediate_at(Actor::System, sub, h.tenant, v2, vec![], day(9, 16))
        .await
        .expect("the operator forces the migration");
    h.sweep(customer, at(9, 16) + Halyard::hours(1)).await;

    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(
        h.movements(paid.id).await,
        vec!["FUND 50000", "DRAW -25000", "REVERSAL -25000"],
        "fee credit min(25000, 50000 × 25000/50000)"
    );
    assert_eq!(
        h.movements(grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000).id)
            .await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    let invoices = h.invoices(sub).await;
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, invoices[0].id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].credit_type, CreditType::CreditToBalance);
    assert_eq!((notes[0].subtotal, notes[0].total), (-25_000, -25_000));
    let note_credit = grant(&grants, "CREDIT_NOTE", "PAID", 25_000);

    let adjustment = &invoices[1];
    assert_eq!(
        money(adjustment),
        (25_000, 0, 0, 0, 25_000, 0, 25_000),
        "v2 pack prorated; the note's credit cannot pay a credit purchase: {:?}",
        lines(adjustment)
    );
    h.check().await;
    h.pay(adjustment.id, at(9, 16)).await;
    let grants = h.grants(customer).await;
    let paid_2 = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    let bonus_2 = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    assert_eq!(
        (paid_2.expires_at, bonus_2.expires_at),
        (Some(at(10, 1)), Some(at(10, 1))),
        "30000 of credit for 25000"
    );
    h.check().await;

    second_half(&h, customer);
    let renewal = h.renew(sub, at(10, 1)).await.expect("the October invoice");
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (100_000, 0, 10_000, 0, 90_000, 40_000, 50_000),
        "{:?}",
        lines(&renewal)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(paid_2.id).await,
        vec!["FUND 25000", "DRAW -15000", "EXPIRY -10000"]
    );
    assert_eq!(
        h.movements(bonus_2.id).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 25_000).remaining,
        25_000
    );
    assert_eq!(note_credit.subscription_id, None);
    assert_welcome_once(&h, customer).await;
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (125_000, 50_000, 75_000, 10_000),
            "deferred = October's pack 50000 + G-cn 25000"
        );
    }
}
