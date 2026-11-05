//! S5. Scale paused on Sep 10 (R29, OQ8): a closing invoice draws the usage to the pause, nothing
//! is billed while paused, and the plan credit keeps its own expiry: the pack lapses on Oct 1 as
//! breakage. Resume is not in this release (FOLLOWUPS "Scenario tests").
use common_domain::actor::Actor;
use diesel_models::enums::SubscriptionStatusEnum;
use diesel_models::subscriptions::SubscriptionRow;
use meteroid_store::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterface;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};

#[tokio::test]
async fn s05_pause_bills_its_usage_and_the_pack_lapses_at_its_expiry() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    let first = first.expect("INV-1");
    h.pay(first.id, at(9, 1)).await;
    // Sep 1-9: standard 15M (18000) + lite 10M (2000) = 20000.
    h.tokens(customer, at_time(9, 3, 10, 0, 0), STANDARD, 15 * M);
    h.tokens(customer, at_time(9, 8, 10, 0, 0), LITE, 10 * M);

    {
        let mut conn = h.store().get_conn().await.expect("connection");
        h.store()
            .schedule_events(
                &mut conn,
                vec![ScheduledEventNew {
                    subscription_id: sub,
                    tenant_id: h.tenant,
                    scheduled_time: day(9, 10).and_time(chrono::NaiveTime::MIN),
                    event_data: ScheduledEventData::PauseSubscription,
                    source: "test".to_owned(),
                    created_by: Actor::System,
                }],
            )
            .await
            .expect("pause on Sep 10");
    }
    h.env.process_due_events().await;
    // The closing invoice finalizes on its own scheduled event.
    h.env.process_due_events().await;

    let row = SubscriptionRow::get_subscription_by_id(&mut h.env.conn().await, &h.tenant, sub)
        .await
        .expect("subscription")
        .subscription;
    assert_eq!(row.status, SubscriptionStatusEnum::Paused);
    assert_eq!(row.current_period_end, None, "the period end is cleared");

    let invoices = h.invoices(sub).await;
    let closing = invoices
        .iter()
        .rev()
        .find(|i| i.line_items.iter().any(|l| l.metric_id.is_some()))
        .expect("the closing invoice");
    assert_eq!(
        money(closing),
        (20_000, 0, 10_000, 0, 10_000, 10_000, 0),
        "usage to the pause, drawn as any invoice draws: bonus and welcome, then paid: {:?} {:?}",
        closing.status,
        lines(closing)
    );
    assert!(
        h.store()
            .list_credit_notes_by_invoice_id(h.tenant, first.id)
            .await
            .expect("notes")
            .is_empty(),
        "a pause credits no pack fee"
    );
    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(11, 30));
    assert_eq!(
        (paid.remaining, bonus.remaining, welcome.remaining),
        (40_000, 0, 0)
    );
    assert!(
        grants.iter().all(|g| g.ends_at.is_none()),
        "a pause ends no credit"
    );
    h.check().await;

    // Oct 1: nothing left to settle; the pack lapses, no renewal and no October pack.
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;
    assert_eq!(
        h.movements(paid.id).await,
        vec!["FUND 50000", "DRAW -10000", "EXPIRY -40000"]
    );
    assert_eq!(
        h.invoices(sub).await.len(),
        invoices.len(),
        "nothing billed"
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (50_000, 50_000, 0, 40_000)
        );
    }
}
