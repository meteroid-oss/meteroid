//! S7. Scale cancelled immediately on Sep 16 with a top-up on the side: the pack settles the
//! usage known then, its fee comes back capped on a note to balance, the subscription's credit
//! ends at the cancellation; a late event within the grace is paid by the welcome credit ending.
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditGrantId, CustomerId, SubscriptionId};
use diesel_models::customer_credit::CustomerCreditEntryRow;
use meteroid_store::domain::{Invoice, InvoicingEntityPatch};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::CreditType;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit};
use uuid::Uuid;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};

/// Movements as "KIND amount", with the reason of reversals and expiries.
async fn ledger(h: &Halyard, grant: CreditGrantId) -> Vec<String> {
    let mut conn = h.env.conn().await;
    CustomerCreditEntryRow::list_by_grant(&mut conn, h.tenant, grant)
        .await
        .expect("movements")
        .into_iter()
        .map(|entry| match entry.reason {
            Some(reason) if entry.kind == "REVERSAL" || entry.kind == "EXPIRY" => {
                format!("{} {} {reason}", entry.kind, entry.amount)
            }
            _ => format!("{} {}", entry.kind, entry.amount),
        })
        .collect()
}

/// The invoice billing the usage up to `on`, the end of the subscription.
fn final_invoice(invoices: &[Invoice], on: chrono::NaiveDate) -> &Invoice {
    invoices
        .iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .any(|line| line.metric_id.is_some() && line.end_date == on)
        })
        .unwrap_or_else(|| {
            panic!(
                "no final invoice on {on}: {:?}",
                invoices.iter().map(lines).collect::<Vec<_>>()
            )
        })
}

async fn entity_grace_hours(h: &Halyard, hours: i32) {
    let entity = h
        .store()
        .get_invoicing_entity(h.tenant, None)
        .await
        .expect("invoicing entity");
    h.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: entity.id,
                grace_period_hours: Some(hours),
                ..Default::default()
            },
            h.tenant,
        )
        .await
        .expect("entity grace");
}

/// Scale from Sep 1 paid (P 50000, B 5000, W 5000), a 10000 top-up bought and paid on Sep 5, and
/// 40000 of usage by Sep 15; the entity bills with a 1 h grace.
async fn on_scale_with_top_up(h: &Halyard) -> (CustomerId, SubscriptionId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    h.pay(first.expect("INV-1").id, at(9, 1)).await;
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "USD")
        .await
        .expect("wallet");
    let sold = h
        .services()
        .sell_customer_credit_at(
            Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet.id,
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
        .expect("top-up sold");
    assert_eq!(
        sold.invoice.invoice.tax_amount, 0,
        "a currency purchase is untaxed"
    );
    h.pay(sold.invoice.invoice.id, at(9, 5)).await;
    let grants = h.grants(customer).await;
    let top_up = grant(&grants, "CREDIT_PURCHASE", "PAID", 10_000);
    assert_eq!(
        (top_up.subscription_id, top_up.expires_at),
        (None, None),
        "T: customer-wide, no expiry"
    );

    // 40000 by Sep 15: standard 25M (30000) + standard 8M (9600) + lite 2M (400).
    h.tokens(customer, at_time(9, 2, 10, 0, 0), STANDARD, 25 * M);
    h.tokens(customer, at_time(9, 8, 10, 0, 0), STANDARD, 8 * M);
    h.tokens(customer, at_time(9, 12, 10, 0, 0), LITE, 2 * M);
    entity_grace_hours(h, 1).await;
    h.check().await;
    (customer, sub)
}

/// Sep 16 00:00: the admin cancels at once. Synchronous settlement (B 5000, P 35000), fee credit
/// min(25000, 50000 × 15000/50000) = 15000 on a note to balance, the plan credit ends then.
async fn cancel_on_sep_16(h: &Halyard, customer: CustomerId, sub: SubscriptionId) {
    h.services()
        .cancel_subscription(
            Actor::System,
            sub,
            h.tenant,
            None,
            CancellationEffectiveAt::Date(day(9, 16)),
        )
        .await
        .expect("cancelled");
    h.env.process_due_events().await;

    let grants = h.grants(customer).await;
    let paid = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(11, 30));
    assert_eq!(
        ledger(h, paid.id).await,
        vec![
            "FUND 50000",
            "DRAW -35000",
            "REVERSAL -15000 Removed: subscription cancelled",
        ]
    );
    assert_eq!(ledger(h, bonus.id).await, vec!["FUND 5000", "DRAW -5000"]);
    for ended in [paid, bonus, welcome] {
        assert_eq!(
            (ended.ends_at, ended.end_reason.as_deref()),
            (Some(at(9, 16)), Some("SUBSCRIPTION_ENDED"))
        );
    }
    assert_eq!(welcome.remaining, 5_000);

    let first = &h.invoices(sub).await[0];
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, first.id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1, "CN-1 on INV-1");
    assert_eq!(notes[0].credit_type, CreditType::CreditToBalance);
    assert_eq!((notes[0].subtotal, notes[0].total), (-15_000, -15_000));
    let note_credit = grant(&grants, "CREDIT_NOTE", "PAID", 15_000);
    assert_eq!(
        (
            note_credit.subscription_id,
            note_credit.expires_at,
            note_credit.remaining
        ),
        (None, None, 15_000)
    );
    let top_up = grant(&grants, "CREDIT_PURCHASE", "PAID", 10_000);
    assert_eq!((top_up.remaining, top_up.ends_at), (10_000, None));
    h.check().await;
}

#[tokio::test]
async fn s07_immediate_termination_settles_known_and_late_usage() {
    let h = Halyard::new().await;
    let (customer, sub) = on_scale_with_top_up(&h).await;
    cancel_on_sep_16(&h, customer, sub).await;

    // Dated Sep 15 23:00, ingested Sep 16 00:30, inside the grace: lite 10M = 2000.
    h.tokens(customer, at_time(9, 15, 23, 0, 0), LITE, 10 * M);
    h.sweep(customer, at_time(9, 16, 1, 30, 0)).await;
    h.env.process_due_events().await;

    let grants = h.grants(customer).await;
    assert_eq!(
        ledger(&h, expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).id).await,
        vec![
            "FUND 5000",
            "DRAW -2000",
            "EXPIRY -3000 Ended with subscription"
        ]
    );
    let invoices = h.invoices(sub).await;
    let last = h.invoice(final_invoice(&invoices, day(9, 16)).id).await;
    assert_eq!(
        money(&last),
        (42_000, 0, 7_000, 0, 35_000, 35_000, 0),
        "INV-F takes both settlements over: B 5000 + W 2000 before tax, P 35000"
    );
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 10_000).remaining,
        10_000
    );
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 15_000).remaining,
        15_000
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(9, 16)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred(), rev.breakage),
            (60_000, 35_000, 25_000, 0),
            "deferred = T 10000 + G-cn 15000; the welcome credit lapses at cost 0"
        );
    }
}

/// Variant: the late event is 8000 (lite 40M). W pays 5000 of it; the 3000 left is billed and
/// paid by T, customer-wide and older than the note's credit.
#[tokio::test]
async fn s07_late_event_beyond_the_ending_credit_is_paid_by_the_top_up() {
    let h = Halyard::new().await;
    let (customer, sub) = on_scale_with_top_up(&h).await;
    cancel_on_sep_16(&h, customer, sub).await;

    h.tokens(customer, at_time(9, 15, 23, 0, 0), LITE, 40 * M);
    h.sweep(customer, at_time(9, 16, 1, 30, 0)).await;
    h.env.process_due_events().await;

    let grants = h.grants(customer).await;
    assert_eq!(
        ledger(&h, expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).id).await,
        vec!["FUND 5000", "DRAW -5000"],
        "nothing left to lapse"
    );
    let invoices = h.invoices(sub).await;
    let last = h.invoice(final_invoice(&invoices, day(9, 16)).id).await;
    assert_eq!(
        money(&last),
        (48_000, 0, 10_000, 0, 38_000, 38_000, 0),
        "P 35000 settled + T 3000"
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 10_000).remaining,
        7_000
    );
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 15_000).remaining,
        15_000
    );
    h.check().await;

    if let Some(rev) = h.revenue(customer, day(9, 16)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (60_000, 38_000, 22_000)
        );
    }
}

/// Self-heal: a cancellation recorded without its terminal hook leaves the plan credit unended;
/// one pass of the repair ends it at the termination and reports the subscription once.
#[tokio::test]
async fn s07_self_heal_ends_the_credit_of_a_subscription_cancelled_without_the_hook() {
    use diesel_async::SimpleAsyncConnection;

    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    h.pay(first.expect("INV-1").id, at(9, 1)).await;
    h.check().await;

    h.env
        .conn()
        .await
        .batch_execute(&format!(
            "UPDATE subscription SET status = 'CANCELLED', end_date = '2025-09-16',
                 terminated_at = '2025-09-16 00:00:00' WHERE id = '{}'",
            sub.as_uuid()
        ))
        .await
        .expect("cancelled without the hook");
    let grants = h.grants(customer).await;
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).ends_at,
        None
    );

    assert_eq!(
        h.store()
            .end_credit_of_ended_subscriptions(100)
            .await
            .expect("repair"),
        1,
        "credit_subscription_end_repaired_total = 1"
    );
    let grants = h.grants(customer).await;
    let welcome = expiring(&grants, "INCLUDED_CREDIT", at(11, 30));
    assert_eq!(
        (welcome.ends_at, welcome.end_reason.as_deref()),
        (Some(at(9, 16)), Some("SUBSCRIPTION_ENDED"))
    );
    assert_eq!(
        h.store()
            .end_credit_of_ended_subscriptions(100)
            .await
            .expect("repair again"),
        0,
        "repaired once"
    );
    h.check().await;
}
