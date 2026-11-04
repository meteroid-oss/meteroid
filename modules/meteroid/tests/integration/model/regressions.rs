//! Engine bugs the model tests found, each pinned by its minimal deterministic sequence.
use chrono::{DateTime, Duration, Utc};
use common_domain::actor::Actor;
use common_domain::ids::CustomerId;
use meteroid_store::domain::Invoice;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use meteroid_store::services::{InvoiceDelivery, OfflineRefundCommand, SellCustomerCredit};

use crate::scenarios::kit::{
    Halyard, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money, summary,
};

/// A purchase of `amount` USD of credit issued at `at`, unpaid.
async fn credit_sale(h: &Halyard, customer: CustomerId, amount: i64, at: DateTime<Utc>) -> Invoice {
    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "USD")
        .await
        .expect("wallet")
        .id;
    h.services()
        .sell_customer_credit_at(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet,
                amount_minor: amount,
                expires_in_months: None,
                note: None,
                actor: Actor::System,
                unit_price: None,
                expected_unit_price: None,
            },
            at,
            InvoiceDelivery::Suppressed,
        )
        .await
        .expect("credit sold")
        .invoice
        .invoice
}

/// A renewal sells October's pack and bills September's usage, 2200 of it paid by a top-up; the
/// top-up pays October's 2400 too. Paid after October was invoiced, the pack is credited back the
/// 2400 its bonus would have paid (OQ11): the note credits the pack line only, which no credit
/// paid, so the top-up gets nothing back.
#[tokio::test]
async fn a_note_crediting_a_pack_gives_nothing_back_to_credit_that_paid_other_lines() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, _) = h.subscribe(customer, h.scale, day(9, 1)).await;
    let top_up = credit_sale(&h, customer, 5_000, at(9, 2)).await;
    h.pay(top_up.id, at(9, 2)).await;
    // Standard 6M = 7200: the welcome credit pays 5000, the top-up 2200.
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 6 * M);
    let october = h.renew(sub, at(10, 1)).await.expect("INV-2");
    assert_eq!(
        money(&october),
        (57_200, 0, 5_000, 0, 52_200, 2_200, 50_000),
        "{:?}",
        lines(&october)
    );
    // Standard 2M = 2400, paid by the top-up on INV-3.
    h.tokens(customer, at_time(10, 10, 10, 0, 0), STANDARD, 2 * M);
    h.renew(sub, at(11, 1)).await.expect("INV-3");

    h.pay(october.id, at(11, 3)).await;
    let notes = h
        .store()
        .list_credit_notes_by_invoice_id(h.tenant, october.id)
        .await
        .expect("notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(
        (notes[0].credit_type, notes[0].total),
        (CreditType::CreditToBalance, -2_400)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        grant(&grants, "CREDIT_NOTE", "PAID", 2_400).remaining,
        2_400,
        "{:?}",
        summary(&grants)
    );
    assert_eq!(
        grant(&grants, "CREDIT_PURCHASE", "PAID", 5_000).remaining,
        400,
        "the 2200 and 2400 it paid on the usage lines stay paid"
    );
    h.check().await;
}

/// Scale's welcome credit, granted on the renewal that switched to Scale, is taken back whole by
/// the void of that invoice: the once credit counts as unbilled and the next renewal grants it.
#[tokio::test]
async fn a_void_taking_the_welcome_credit_back_whole_lets_the_next_renewal_grant_it() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, _) = h.subscribe(customer, h.payg, day(8, 6)).await;
    h.services()
        .schedule_plan_change(Actor::System, sub, h.tenant, h.scale, vec![])
        .await
        .expect("the change at the period end");
    let switched = h.renew(sub, at(9, 6)).await.expect("INV-1");
    let welcome = expiring(&h.grants(customer).await, "INCLUDED_CREDIT", at(12, 5)).id;
    h.store()
        .void_invoice_at(
            Actor::System,
            switched.id,
            h.tenant,
            at_time(9, 6, 14, 0, 0),
        )
        .await
        .expect("voided");
    assert_eq!(
        h.movements(welcome).await,
        vec!["FUND 5000", "REVERSAL -5000"],
        "taken back whole"
    );

    let october = h.renew(sub, at(10, 6)).await.expect("INV-2");
    let grants = h.grants(customer).await;
    let again = expiring(&grants, "INCLUDED_CREDIT", at(10, 6) + Duration::days(90));
    assert_eq!(
        (again.amount, again.remaining, again.subscription_id),
        (5_000, 5_000, Some(sub)),
        "{:?}",
        lines(&october)
    );
    h.check().await;
}

/// A void at a past business moment gives back what the invoice drew and lapses only the credit
/// due by then: the period pack paid on Sep 1 is still spendable on Sep 10.
#[tokio::test]
async fn a_void_at_a_past_moment_lapses_only_credit_due_by_then() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (_, first) = h.subscribe(customer, h.scale, day(9, 1)).await;
    h.pay(first.expect("INV-1").id, at(9, 1)).await;
    let sale = credit_sale(&h, customer, 5_000, at(9, 5)).await;

    h.store()
        .void_invoice_at(Actor::System, sale.id, h.tenant, at_time(9, 10, 14, 0, 0))
        .await
        .expect("voided");
    let grants = h.grants(customer).await;
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(h.movements(pack.id).await, vec!["FUND 50000"]);
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(11, 30)).remaining,
        5_000
    );
    h.check().await;
}

/// A refund note dates its reversal when it is made (now), so the renewal's draw on the same
/// top-up is dated then too; a void at a past business moment gives that draw back no earlier
/// than it, so the top-up's running balance never exceeds its face.
#[tokio::test]
async fn a_void_gives_credit_back_no_earlier_than_the_draw_it_returns() {
    let h = Halyard::new().await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, _) = h.subscribe(customer, h.payg, day(8, 13)).await;
    h.tokens(customer, at_time(8, 13, 14, 0, 0), STANDARD, 15 * M);
    let top_up = credit_sale(&h, customer, 10_000, at(8, 14)).await;
    let top_up = h.pay(top_up.id, at(8, 14)).await;
    let note = h
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            h.tenant,
            CreateCreditNoteParams {
                invoice_id: top_up.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: top_up.line_items[0].local_id.clone(),
                    amount_cents: 5_001,
                }],
                reason: Some("Refund".into()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .expect("refund note");
    let payment = h
        .store()
        .list_payment_tx_by_invoice_id(h.tenant, top_up.id)
        .await
        .expect("payments")
        .into_iter()
        .map(|tx| tx.transaction.id)
        .next()
        .expect("the payment");
    h.services()
        .record_offline_refund(
            Actor::System,
            h.tenant,
            OfflineRefundCommand {
                parent_transaction_id: payment,
                amount: 5_001,
                credit_note_id: Some(note.id),
                refund_date: at(8, 15).naive_utc(),
                reference: Some("wire".to_owned()),
                reason: None,
            },
        )
        .await
        .expect("refunded");
    let september = h.renew(sub, at(9, 13)).await.expect("INV-2");
    assert_eq!(september.applied_credits, 4_999);

    h.store()
        .void_invoice_at(
            Actor::System,
            september.id,
            h.tenant,
            at_time(9, 13, 14, 0, 0),
        )
        .await
        .expect("voided");
    assert_eq!(
        grant(&h.grants(customer).await, "CREDIT_PURCHASE", "PAID", 10_000).remaining,
        4_999
    );
    h.check().await;
}
