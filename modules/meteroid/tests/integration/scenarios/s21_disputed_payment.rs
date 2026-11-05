//! S21. S14's overpaid adjustment invoice (25000 asked, 30000 collected, OVERPAYMENT 5000) lost
//! to the payer: a chargeback keeps the invoice paid and takes back the credit the money funded,
//! and a won dispute restores the same grants. A reversal never takes more than the payment still
//! holds: after a 5000 wire refund, a full chargeback takes 25000 and a late failure reopens the
//! invoice for 25000 only, so a replacement keeps the true excess.
use common_domain::actor::Actor;
use meteroid_store::domain::InvoicePaymentStatus;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::services::OfflineRefundCommand;

use super::kit::{Halyard, at, grant};
use super::s13_top_up_void_note_refund::one_off;
use super::s14_overpayment::{
    charge_back, collect, fail_late, overpaid, overpayment, provider_payment, reinstate,
};
use crate::helpers::invoice_lines::line;

/// Wires 5000 of the 30000 back to the customer by hand, without a credit note: the excess.
async fn refund_the_excess_by_wire(h: &Halyard, invoice: common_domain::ids::InvoiceId) {
    let payment = h
        .store()
        .list_payment_tx_by_invoice_id(h.tenant, invoice)
        .await
        .expect("payments")
        .into_iter()
        .find(|tx| tx.transaction.amount == 30_000)
        .expect("the provider payment")
        .transaction
        .id;
    h.services()
        .record_offline_refund(
            Actor::System,
            h.tenant,
            OfflineRefundCommand {
                parent_transaction_id: payment,
                amount: 5_000,
                credit_note_id: None,
                refund_date: at(9, 20).naive_utc(),
                reference: Some("wire-out".to_owned()),
                reason: None,
            },
        )
        .await
        .expect("the excess wired back");
}

async fn reversed_of(h: &Halyard, invoice: common_domain::ids::InvoiceId) -> i64 {
    h.store()
        .list_payment_tx_by_invoice_id(h.tenant, invoice)
        .await
        .expect("payments")
        .into_iter()
        .find(|tx| tx.transaction.amount == 30_000)
        .expect("the provider payment")
        .transaction
        .amount_reversed
}

#[tokio::test]
async fn s21_a_charged_back_invoice_stays_paid_and_a_won_dispute_restores_the_same_grants() {
    let h = Halyard::new().await;
    let (customer, _, adjustment) = overpaid(&h).await;
    let external_id = provider_payment(&h, adjustment).await;

    charge_back(&h, &external_id, at(9, 22)).await;
    let disputed = h.invoice(adjustment).await;
    assert_eq!(
        (disputed.amount_due, disputed.payment_status),
        (0, InvoicePaymentStatus::Paid),
        "a payment lost to the payer never makes the invoice owed again"
    );
    let grants = h.grants(customer).await;
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 2_500);
    assert_eq!(
        h.movements(overpayment(&grants).id).await,
        vec!["FUND 5000", "REVERSAL -5000"]
    );
    assert_eq!(
        h.movements(pack.id).await,
        vec!["FUND 25000", "REVERSAL -25000"],
        "the credit the lost money bought is taken back"
    );
    assert_eq!(
        h.movements(bonus.id).await,
        vec!["FUND 2500", "REVERSAL -2500"]
    );
    h.check().await;

    reinstate(&h, &external_id, at(9, 24)).await;
    let grants = h.grants(customer).await;
    assert_eq!(overpayment(&grants).remaining, 5_000);
    assert_eq!(
        h.movements(pack.id).await,
        vec!["FUND 25000", "REVERSAL -25000", "REINSTATEMENT 25000"]
    );
    assert_eq!(
        h.movements(bonus.id).await,
        vec!["FUND 2500", "REVERSAL -2500", "REINSTATEMENT 2500"]
    );
    let won = h.invoice(adjustment).await;
    assert_eq!(
        (won.amount_due, won.payment_status),
        (0, InvoicePaymentStatus::Paid)
    );
    h.check().await;
}

/// Seed 0xc0fff2 on S14: 5000 wired back, then the provider takes all 30000. Only the
/// 25000 the payment still held is applied; the 5000 above it is the merchant's to contest.
#[tokio::test]
async fn s21_a_full_chargeback_after_a_wire_refund_takes_only_what_the_payment_held() {
    let h = Halyard::new().await;
    let (customer, _, adjustment) = overpaid(&h).await;
    let external_id = provider_payment(&h, adjustment).await;
    refund_the_excess_by_wire(&h, adjustment).await;
    let grants = h.grants(customer).await;
    assert_eq!(
        overpayment(&grants).remaining,
        0,
        "the wire gave the excess back"
    );

    charge_back(&h, &external_id, at(9, 22)).await;
    assert_eq!(
        reversed_of(&h, adjustment).await,
        25_000,
        "30000 collected, 5000 refunded: 25000 was left to lose"
    );
    let disputed = h.invoice(adjustment).await;
    assert_eq!(
        (disputed.amount_due, disputed.payment_status),
        (0, InvoicePaymentStatus::Paid)
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(overpayment(&grants).id).await,
        vec!["FUND 5000", "REVERSAL -5000"],
        "the excess left with the wire, not with the chargeback"
    );
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    assert_eq!(
        h.movements(pack.id).await,
        vec!["FUND 25000", "REVERSAL -25000"]
    );
    h.check().await;
}

/// The same refund, then the debit fails late: the invoice reopens for the 25000 the payment held,
/// and a 26000 replacement keeps 1000 as OVERPAYMENT credit, the money truly above the invoice.
#[tokio::test]
async fn s21_a_late_failure_after_a_wire_refund_reopens_only_what_the_payment_held() {
    let h = Halyard::new().await;
    let (customer, _, adjustment) = overpaid(&h).await;
    let external_id = provider_payment(&h, adjustment).await;
    refund_the_excess_by_wire(&h, adjustment).await;

    fail_late(&h, &external_id, at(9, 22)).await;
    assert_eq!(reversed_of(&h, adjustment).await, 25_000);
    let reopened = h.invoice(adjustment).await;
    assert_eq!(
        (reopened.amount_due, reopened.payment_status),
        (25_000, InvoicePaymentStatus::Unpaid)
    );
    h.check().await;

    collect(&h, adjustment, 26_000, at(9, 24)).await;
    let paid = h.invoice(adjustment).await;
    assert_eq!(
        (paid.amount_due, paid.payment_status),
        (0, InvoicePaymentStatus::Paid)
    );
    let kept: i64 = h
        .grants(customer)
        .await
        .iter()
        .filter(|g| g.source_kind == "OVERPAYMENT")
        .map(|g| g.remaining)
        .sum();
    assert_eq!(
        kept, 1_000,
        "30000 - 5000 wired - 25000 failed + 26000 collected = 26000 for 25000 asked"
    );
    h.check().await;
}

/// 2000 of S14's overpayment credit spent on a one-off, then the payment charged back: the
/// dispute books the spent 2000 as lost. Voiding the one-off gives the 2000 back and the dispute
/// takes it at once, which recovers that loss: revenue does not move, nothing is booked twice.
#[tokio::test]
async fn s21_credit_given_back_after_a_dispute_and_taken_again_is_not_booked_twice() {
    let h = Halyard::new().await;
    let (customer, _, adjustment) = overpaid(&h).await;
    let spent = one_off(&h, customer, vec![line("Support", 2_000, false)], at(9, 20)).await;
    let external_id = provider_payment(&h, adjustment).await;
    charge_back(&h, &external_id, at(9, 22)).await;
    let today = chrono::Utc::now().date_naive();
    let before = h.revenue(customer, today).await;

    h.store()
        .void_invoice(Actor::System, spent.id, h.tenant)
        .await
        .expect("the one-off voided");
    let grants = h.grants(customer).await;
    assert_eq!(
        overpayment(&grants).remaining,
        0,
        "given back to credit the lost money paid, taken back at once"
    );
    if let (Some(before), Some(after)) = (before, h.revenue(customer, today).await) {
        assert_eq!(
            (after.billed, after.recognized),
            (before.billed, before.recognized),
            "the credit taken back recovers the dispute's loss on it"
        );
    }
    h.check().await;
}
