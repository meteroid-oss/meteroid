//! S14. An upgrade's adjustment invoice, sent for payment, is overpaid through the provider; the
//! excess is kept as OVERPAYMENT credit (OQ12). A late failure of the payment takes it and the
//! pack back and reopens the invoice, and a replacement payment reinstates the same grants; a
//! chargeback takes them back too but leaves the invoice paid (see S21). A manual payment above
//! the amount due is refused, so the money arrives as provider settlements dated by their events.
use chrono::{DateTime, Duration, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, PaymentTransactionId, SubscriptionId};
use diesel_models::customer_credit::CustomerCreditGrantRow;
use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::PaymentTransactionRowNew;
use http::HeaderMap;
use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::InvoicePaymentStatus;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

use super::kit::{
    Halyard, LITE, M, STANDARD, Tax, at, at_time, day, expiring, grant, lines, money,
};
use super::s13_top_up_void_note_refund::{one_off, unrecovered};
use crate::helpers::invoice_lines::line;

/// Runs a mock provider webhook through the real dispatcher, as the gateway delivers it.
pub(super) async fn deliver(h: &Halyard, payload: String) {
    let connector = h
        .store()
        .get_connector_with_data_by_alias("sandbox-gateway".to_owned(), h.tenant)
        .await
        .expect("the sandbox gateway");
    let provider = initialize_payment_connector(&connector).expect("mock connector");
    let event = provider
        .parse_event(payload.as_bytes(), &HeaderMap::new())
        .expect("parsed")
        .expect("an event");
    handle_normalized_event(
        event,
        &connector,
        provider.as_ref(),
        h.store().clone(),
        h.services(),
    )
    .await
    .expect("handled");
    h.env.run_outbox_and_orchestration().await;
}

/// A provider payment of `amount` for the invoice, collected at `at`; returns its external id.
pub(super) async fn collect(
    h: &Halyard,
    invoice: InvoiceId,
    amount: i64,
    at: DateTime<Utc>,
) -> String {
    let payment = PaymentTransactionId::new();
    let external_id = format!("pi_{}", payment.as_base62());
    PaymentTransactionRowNew {
        id: payment,
        tenant_id: h.tenant,
        invoice_id: Some(invoice),
        provider_transaction_id: Some(external_id.clone()),
        amount,
        currency: "USD".to_owned(),
        payment_method_id: None,
        status: PaymentStatusEnum::Pending,
        payment_type: PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: None,
        checkout_session_id: None,
        pending_plan_version_id: None,
        next_action: None,
        initiated_by_customer_id: None,
        pending_provider_intent_id: None,
        pending_connection_id: None,
        parent_transaction_id: None,
        credit_note_id: None,
        refund_mode: None,
        reversal_reason: None,
    }
    .insert(&mut h.env.conn().await)
    .await
    .expect("a pending provider payment");
    deliver(
        h,
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{external_id}","amount":{amount},"currency":"USD","occurred_at":"{at}"}}"#,
            tx = payment.as_base62(),
            at = at.to_rfc3339(),
        ),
    )
    .await;
    external_id
}

pub(super) async fn charge_back(h: &Halyard, external_id: &str, at: DateTime<Utc>) {
    deliver(
        h,
        format!(
            r#"{{"id":"evt_cb_{external_id}","kind":"payment_charged_back","external_id":"{external_id}","occurred_at":"{at}"}}"#,
            at = at.to_rfc3339(),
        ),
    )
    .await;
}

/// The bank reports the payment's debit failed after it settled (GoCardless
/// `late_failure_settled`): a failed collection, the invoice reopens.
pub(super) async fn fail_late(h: &Halyard, external_id: &str, at: DateTime<Utc>) {
    deliver(
        h,
        format!(
            r#"{{"id":"evt_lf_{external_id}","kind":"payment_late_failure","external_id":"{external_id}","occurred_at":"{at}"}}"#,
            at = at.to_rfc3339(),
        ),
    )
    .await;
}

/// The payer's bank cancels the chargeback (GoCardless `chargeback_cancelled`): the money returns.
pub(super) async fn reinstate(h: &Halyard, external_id: &str, at: DateTime<Utc>) {
    deliver(
        h,
        format!(
            r#"{{"id":"evt_ri_{external_id}","kind":"payment_reinstated","external_id":"{external_id}","occurred_at":"{at}"}}"#,
            at = at.to_rfc3339(),
        ),
    )
    .await;
}

/// The provider id of the payment collected on `invoice`.
pub(super) async fn provider_payment(h: &Halyard, invoice: InvoiceId) -> String {
    h.store()
        .list_payment_tx_by_invoice_id(h.tenant, invoice)
        .await
        .expect("payments")
        .into_iter()
        .find_map(|tx| tx.transaction.provider_transaction_id)
        .expect("the provider payment")
}

pub(super) fn overpayment(grants: &[CustomerCreditGrantRow]) -> &CustomerCreditGrantRow {
    let kept: Vec<_> = grants
        .iter()
        .filter(|g| g.source_kind == "OVERPAYMENT")
        .collect();
    assert_eq!(kept.len(), 1, "one OVERPAYMENT grant keeps the excess");
    kept[0]
}

/// PAYG from Sep 1 (send invoice), changed to Scale on Sep 16; INV-A overpaid 30000 on Sep 18.
pub(super) async fn overpaid(h: &Halyard) -> (CustomerId, SubscriptionId, InvoiceId) {
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, _) = h.subscribe(customer, h.payg, day(9, 1)).await;
    let change = h
        .services()
        .apply_plan_change_immediate_at(Actor::System, sub, h.tenant, h.scale, vec![], day(9, 16))
        .await
        .expect("an upgrade applies at once");
    let adjustment = h
        .invoice(change.adjustment_invoice_id.expect("INV-A"))
        .await;
    assert_eq!(
        money(&adjustment),
        (25_000, 0, 0, 0, 25_000, 0, 25_000),
        "Scale's pack for 15 of 30 days: {:?}",
        lines(&adjustment)
    );
    let grants = h.grants(customer).await;
    assert!(
        grants.iter().all(|g| g.source_kind != "CREDIT_PURCHASE"),
        "the pack is not minted unpaid"
    );
    assert_eq!(
        expiring(&grants, "INCLUDED_CREDIT", at(12, 15)).amount,
        5_000,
        "the welcome credit from Sep 16, for 90 days"
    );
    h.check().await;

    collect(h, adjustment.id, 30_000, at(9, 18)).await;
    let paid = h.invoice(adjustment.id).await;
    assert_eq!(
        (paid.amount_due, paid.payment_status),
        (0, InvoicePaymentStatus::Paid)
    );
    let grants = h.grants(customer).await;
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 2_500);
    assert_eq!(
        (pack.effective_at, pack.expires_at),
        (at(9, 18), Some(at(10, 1)))
    );
    assert_eq!(bonus.expires_at, Some(at(10, 1)));
    let excess = overpayment(&grants);
    assert_eq!(
        (
            excess.classification.as_str(),
            excess.amount,
            excess.subscription_id,
            excess.application.as_str(),
            excess.expires_at,
            excess.effective_at,
        ),
        ("PAID", 5_000, None, "INVOICE", None, at(9, 18))
    );
    h.check().await;
    (customer, sub, adjustment.id)
}

#[tokio::test]
async fn s14_overpayment_failed_late_then_replaced_reinstates_the_same_grants() {
    let h = Halyard::new().await;
    let (customer, sub, adjustment) = overpaid(&h).await;
    let external_id = provider_payment(&h, adjustment).await;

    fail_late(&h, &external_id, at(9, 22)).await;
    let grants = h.grants(customer).await;
    let excess = overpayment(&grants);
    assert_eq!(
        h.movements(excess.id).await,
        vec!["FUND 5000", "REVERSAL -5000"]
    );
    let pack = grant(&grants, "CREDIT_PURCHASE", "PAID", 25_000);
    let bonus = grant(&grants, "CREDIT_PURCHASE", "PROMOTIONAL", 2_500);
    assert_eq!(
        h.movements(pack.id).await,
        vec!["FUND 25000", "REVERSAL -25000"]
    );
    assert_eq!(
        h.movements(bonus.id).await,
        vec!["FUND 2500", "REVERSAL -2500"]
    );
    assert_eq!(h.invoice(adjustment).await.amount_due, 25_000);
    h.check().await;

    collect(&h, adjustment, 25_000, at(9, 24)).await;
    let grants = h.grants(customer).await;
    assert_eq!(
        overpayment(&grants).remaining,
        0,
        "nothing above the amount due"
    );
    assert_eq!(
        grants
            .iter()
            .filter(|g| g.source_kind == "CREDIT_PURCHASE")
            .count(),
        2,
        "the same grants come back, none is minted again"
    );
    assert_eq!(
        h.movements(pack.id).await,
        vec!["FUND 25000", "REVERSAL -25000", "REINSTATEMENT 25000"]
    );
    assert_eq!(
        h.movements(bonus.id).await,
        vec!["FUND 2500", "REVERSAL -2500", "REINSTATEMENT 2500"]
    );
    h.check().await;

    // Sep 24-30: standard 15M (18000) + lite 25M (5000) = 23000.
    h.tokens(customer, at_time(9, 25, 10, 0, 0), STANDARD, 15 * M);
    h.tokens(customer, at_time(9, 28, 10, 0, 0), LITE, 25 * M);
    let renewal = h.renew(sub, at(10, 1)).await.expect("INV-2");
    h.sweep(customer, at(10, 1) + Duration::hours(1)).await;
    let renewal = h.invoice(renewal.id).await;
    assert_eq!(
        money(&renewal),
        (73_000, 0, 2_500, 0, 70_500, 20_500, 50_000),
        "{:?}",
        lines(&renewal)
    );
    assert_eq!(
        h.movements(pack.id).await,
        vec![
            "FUND 25000",
            "REVERSAL -25000",
            "REINSTATEMENT 25000",
            "DRAW -20500",
            "EXPIRY -4500"
        ]
    );
    assert_eq!(
        expiring(&h.grants(customer).await, "INCLUDED_CREDIT", at(12, 15)).remaining,
        5_000
    );
    h.check().await;

    // The reversal and the replacement are dated at their events, so the revenue is exact as of
    // October 1 and not only once read after them.
    if let Some(rev) = h.revenue(customer, day(10, 1)).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (75_000, 25_000, 50_000),
            "the overpayment nets to nothing billed"
        );
    }
    if let Some(rev) = h.revenue(customer, chrono::Utc::now().date_naive()).await {
        assert_eq!(
            (rev.billed, rev.recognized, rev.deferred()),
            (75_000, 25_000, 50_000)
        );
    }
}

/// 2000 of the excess spent on a one-off invoice before a chargeback: 3000 is taken back, the
/// 2000 spent is reported lost to the dispute like a purchase's, and the invoice stays paid.
#[tokio::test]
async fn s14_spent_overpayment_is_reported_unrecovered() {
    let h = Halyard::new().await;
    let (customer, _, adjustment) = overpaid(&h).await;
    let spent = one_off(&h, customer, vec![line("Support", 2_000, false)], at(9, 20)).await;
    assert_eq!(money(&spent), (2_000, 0, 0, 0, 2_000, 2_000, 0));
    let external_id = provider_payment(&h, adjustment).await;

    charge_back(&h, &external_id, at(9, 22)).await;
    let grants = h.grants(customer).await;
    assert_eq!(
        h.movements(overpayment(&grants).id).await,
        vec!["FUND 5000", "DRAW -2000", "REVERSAL -3000"]
    );
    assert_eq!(
        unrecovered(&h, adjustment).await,
        vec![("PAYMENT_DISPUTED".to_owned(), 2_000, 2_000)]
    );
    let disputed = h.invoice(adjustment).await;
    assert_eq!(
        (disputed.amount_due, disputed.payment_status),
        (0, InvoicePaymentStatus::Paid),
        "a chargeback never reopens the invoice"
    );
    h.check().await;
}
