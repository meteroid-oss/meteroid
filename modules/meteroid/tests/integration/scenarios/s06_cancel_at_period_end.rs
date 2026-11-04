//! S6. Scale cancelled on Sep 20 for the period end. On Oct 1 its pack, bonus and welcome credit
//! end with it and, lapsing at that bound, pay September's usage first (promotional, then paid);
//! the rest of the pack is breakage, "Ended with subscription" (OQ18). The top-up is untouched.
use common_domain::actor::Actor;
use common_domain::ids::CreditGrantId;
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use meteroid_store::domain::LineItemKind;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::wallet_events::{CreditGrantExpiredEvent, GrantLapseReason};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};

async fn expired_events(h: &Halyard, grant: CreditGrantId) -> Vec<CreditGrantExpiredEvent> {
    h.store()
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
            OutboxEvent::CreditGrantExpired(expired) if expired.credit_grant_id == grant => {
                Some(*expired)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn s06_cancel_at_period_end_lapses_the_plan_credit_with_its_subscription() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    h.pay(first.expect("INV-1").id, at(9, 1)).await;

    // Sep 5: a top-up of 10000, customer-wide, never expiring.
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "USD")
        .await
        .expect("wallet")
        .id;
    let sold = h
        .services()
        .sell_customer_credit_at(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet,
                amount_minor: 10_000,
                expires_in_months: None,
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            at(9, 5),
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("the top-up");
    assert_eq!(
        money(&sold.invoice.invoice),
        (10_000, 0, 0, 0, 10_000, 0, 10_000),
        "a credit purchase is untaxed"
    );
    h.pay(sold.invoice.invoice.id, at(9, 5)).await;
    let grants = h.grants(customer).await;
    let top_up = grant(&grants, "CREDIT_PURCHASE", "PAID", 10_000).id;

    // September: standard 30M (36000) + lite 20M (4000) = 40000.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 30 * M);
    h.tokens(customer, at_time(9, 25, 10, 0, 0), LITE, 20 * M);

    h.services()
        .cancel_subscription(
            Actor::System,
            sub,
            h.tenant,
            None,
            CancellationEffectiveAt::Date(day(10, 1)),
        )
        .await
        .expect("cancelled for the period end");
    let details = h
        .store()
        .get_subscription_details(h.tenant, sub)
        .await
        .expect("details");
    let preview = h
        .services()
        .compute_upcoming_invoice(&details)
        .await
        .expect("the final invoice's preview");
    assert!(
        preview
            .invoice_lines
            .iter()
            .all(|line| line.kind != LineItemKind::Credit),
        "the final invoice sells no October pack"
    );
    assert!(
        h.store()
            .list_credit_notes_by_invoice_id(h.tenant, h.invoices(sub).await[0].id)
            .await
            .expect("notes")
            .is_empty(),
        "nothing unused to credit"
    );
    h.check().await;

    h.renew(sub, at(10, 1)).await;
    h.env.process_due_events().await;
    h.sweep(customer, at(10, 1) + Halyard::hours(1)).await;

    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(11, 30));
    for ended in [paid, bonus, welcome] {
        assert_eq!(
            (ended.ends_at, ended.end_reason.as_deref()),
            (Some(at(10, 1)), Some("SUBSCRIPTION_ENDED"))
        );
    }
    assert_eq!(
        h.movements_with_reasons(paid.id).await,
        vec![
            "FUND 50000",
            "DRAW -30000",
            "EXPIRY -20000 Ended with subscription"
        ]
    );
    assert_eq!(h.movements(bonus.id).await, vec!["FUND 5000", "DRAW -5000"]);
    assert_eq!(
        h.movements(welcome.id).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    let top_up_row = grants.iter().find(|g| g.id == top_up).expect("the top-up");
    assert_eq!((top_up_row.remaining, top_up_row.ends_at), (10_000, None));

    let invoices = h.invoices(sub).await;
    let last = invoices
        .iter()
        .rev()
        .find(|i| i.line_items.iter().any(|l| l.metric_id.is_some()))
        .expect("the final invoice");
    assert_eq!(
        money(last),
        (40_000, 0, 10_000, 0, 30_000, 30_000, 0),
        "{:?}",
        lines(last)
    );
    let expired = expired_events(&h, paid.id).await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].reason, GrantLapseReason::SubscriptionEnded);
    assert_eq!(expired[0].subscription_id, Some(sub));
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (60_000, 50_000, 10_000, 20_000),
            "deferred = the top-up"
        );
    }
}
