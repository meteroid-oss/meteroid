//! Refunds: giving settled money back on purpose, and the claw-back that isn't.
//!
//! A refund is a child REFUND row on a settled payment. It never moves the invoice's
//! `amount_due` or payment status — what reduces the invoice is the finalized `Refund` credit
//! note the refund is drawn against, the only exception being an overpayment. An involuntary
//! reversal (chargeback, bank recall, failed collection) is the other mechanism entirely: it
//! mutates the parent, keeps the invoice paid for a dispute and reopens it for a failed
//! collection, and is covered in `payment_webhook_settlement.rs`.
//!
//! Provider-side refunds are driven through the mock adapter's own webhook envelope, so these
//! exercise the production dispatcher exactly as the HTTP webhook route does.

use chrono::NaiveDate;
use rstest::rstest;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, test_env};

use common_domain::actor::Actor;
use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId, SubscriptionId};
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::connectors::MockRefundBehavior;
use meteroid_store::domain::credit_notes::CreditNote;
use meteroid_store::domain::enums::{
    CreditNoteStatus, InvoicePaymentStatus, PaymentTypeEnum, RefundMode, ReversalKind,
};
use meteroid_store::domain::payment_transactions::PaymentTransaction;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::services::{OfflineRefundCommand, RefundCommand, ReverseManualPaymentCommand};

fn actor() -> Actor {
    Actor::User { id: USER_ID }
}

fn now() -> chrono::NaiveDateTime {
    chrono::Utc::now().naive_utc()
}

/// Stripe `charge.refunded`-style envelope: the charge's cumulative refunded total, plus the
/// refund that triggered it.
fn refunded_payload(external_id: &str, cumulative_minor: i64, refund_id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_re_{amt}","kind":"payment_refunded","external_id":"{ext}","amount":{amt},"currency":"EUR","refunds":[{{"id":"{re}","amount":{amt},"succeeded":true}}]}}"#,
        ext = external_id,
        amt = cumulative_minor,
        re = refund_id,
    )
    .into_bytes()
}

/// The REFUND children recorded against `parent_id`, oldest first.
async fn refunds_of(env: &TestEnv, parent_id: PaymentTransactionId) -> Vec<PaymentTransaction> {
    use diesel_models::payments::PaymentTransactionRow;
    let mut conn = env.conn().await;
    PaymentTransactionRow::list_refunds_by_parent(&mut conn, parent_id, TENANT_ID)
        .await
        .expect("list refunds of a transaction")
        .into_iter()
        .map(Into::into)
        .collect()
}

async fn payment(env: &TestEnv, tx_id: PaymentTransactionId) -> PaymentTransaction {
    env.store()
        .get_payment_tx_by_id(TENANT_ID, tx_id)
        .await
        .expect("get payment transaction")
}

/// A finalized `Refund` credit note for `amount_cents` drawn on the invoice's first line — the
/// avoir a refund must be backed by.
async fn refund_credit_note(env: &TestEnv, invoice_id: InvoiceId, amount_cents: i64) -> CreditNote {
    let invoice = env.get_detailed_invoice(invoice_id).await.invoice;
    let local_id = invoice
        .line_items
        .first()
        .expect("the invoice has a line to credit")
        .local_id
        .clone();

    let credit_note = env
        .store()
        .create_and_finalize_credit_note(
            actor(),
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id,
                line_items: vec![CreditLineItem::Amount {
                    local_id,
                    amount_cents,
                }],
                reason: Some("Goodwill".to_string()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .expect("create and finalize a refund credit note");

    assert_eq!(credit_note.status, CreditNoteStatus::Finalized);
    assert_eq!(credit_note.refunded_amount_cents, amount_cents);
    credit_note
}

/// A finalized invoice paid in full by a manual (wire) payment: a parent with no payment
/// method, which no provider can refund.
async fn manually_paid_invoice(env: &TestEnv) -> (SubscriptionId, InvoiceId, PaymentTransaction) {
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    let tx = env
        .services()
        .add_manual_payment_transaction(
            actor(),
            TENANT_ID,
            invoice_id,
            dec!(35),
            now().and_utc().fixed_offset(),
            Some("wire-in-1".to_string()),
        )
        .await
        .expect("manual payment recorded");
    env.run_outbox_and_orchestration().await;

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
    (sub_id, invoice_id, tx)
}

// =============================================================================
// Online refunds through the provider
// =============================================================================

/// The nominal path: a refund drawn on a finalized avoir settles at the provider, is recorded as
/// a child of the payment, and leaves the invoice exactly as it was.
#[rstest]
#[tokio::test]
async fn test_online_refund_settles_and_leaves_the_invoice_paid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;

    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;

    let refund = env
        .services()
        .refund_transaction(
            actor(),
            TENANT_ID,
            RefundCommand {
                parent_transaction_id: tx.id,
                amount: 1000,
                credit_note_id: Some(credit_note.id),
                reason: Some("Goodwill".to_string()),
            },
        )
        .await
        .expect("the mock provider accepts the refund");

    assert_eq!(refund.status, PaymentStatusEnum::Settled);
    assert_eq!(refund.payment_type, PaymentTypeEnum::Refund);
    assert_eq!(refund.amount, 1000);
    assert_eq!(refund.refund_mode, Some(RefundMode::Online));
    assert_eq!(refund.parent_transaction_id, Some(tx.id));
    assert_eq!(refund.credit_note_id, Some(credit_note.id));
    assert_eq!(refund.invoice_id, Some(invoice_id));
    assert!(
        refund
            .provider_transaction_id
            .as_deref()
            .is_some_and(|id| id.starts_with("mock_re_")),
        "the settled refund keeps the provider's refund id: {:?}",
        refund.provider_transaction_id
    );

    let parent = payment(&env, tx.id).await;
    assert_eq!(parent.status, PaymentStatusEnum::Settled);
    assert_eq!(parent.amount_refunded, 1000);
    assert_eq!(parent.amount_reversed, 0, "a refund is not a claw-back");

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);

    let summary = env
        .store()
        .get_credit_note_refunds(TENANT_ID, credit_note.id)
        .await
        .expect("credit note refund summary");
    assert_eq!(summary.refunded_cents, 1000);
    assert_eq!(summary.pending_cents, 0);
    assert_eq!(summary.refundable_cents, 0);
    assert_eq!(summary.refunds.len(), 1);
}

/// A refund the provider only acknowledges settles when its webhook arrives; until then it is
/// reserved against the credit note but not counted as given back.
#[rstest]
#[tokio::test]
async fn test_pending_online_refund_settles_on_the_provider_webhook(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;
    env.set_mock_refund_behavior(MockRefundBehavior::Pending)
        .await;

    let refund = env
        .services()
        .refund_transaction(
            actor(),
            TENANT_ID,
            RefundCommand {
                parent_transaction_id: tx.id,
                amount: 1000,
                credit_note_id: Some(credit_note.id),
                reason: None,
            },
        )
        .await
        .expect("the provider acknowledged the refund");

    assert_eq!(refund.status, PaymentStatusEnum::Pending);
    let refund_id = refund
        .provider_transaction_id
        .clone()
        .expect("an acknowledged refund carries the provider's refund id");
    assert_eq!(
        payment(&env, tx.id).await.amount_refunded,
        0,
        "money in flight is not money given back"
    );

    let summary = env
        .store()
        .get_credit_note_refunds(TENANT_ID, credit_note.id)
        .await
        .expect("credit note refund summary");
    assert_eq!(summary.pending_cents, 1000);
    assert_eq!(summary.refundable_cents, 0, "the draw is already reserved");

    env.deliver_mock_webhook(&refunded_payload(&external_id, 1000, &refund_id))
        .await;

    let refunds = refunds_of(&env, tx.id).await;
    assert_eq!(
        refunds.len(),
        1,
        "the webhook settles the row we issued instead of recording a second one"
    );
    assert_eq!(refunds[0].id, refund.id);
    assert_eq!(refunds[0].status, PaymentStatusEnum::Settled);
    assert!(refunds[0].processed_at.is_some());
    assert_eq!(payment(&env, tx.id).await.amount_refunded, 1000);

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
}

/// A 1000 refund is in flight when the bank returns the whole debit: only what the payment still
/// holds is reversed, 1000 is kept as withdrawn above it. The refund then fails: that 1000 is
/// applied, the invoice owes exactly 1000 more, and a redelivered failure changes nothing.
#[rstest]
#[tokio::test]
async fn test_a_failed_refund_applies_what_a_reversal_withdrew_above_the_payment(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();
    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;
    env.set_mock_refund_behavior(MockRefundBehavior::Pending)
        .await;
    let refund = env
        .services()
        .refund_transaction(
            actor(),
            TENANT_ID,
            RefundCommand {
                parent_transaction_id: tx.id,
                amount: 1000,
                credit_note_id: Some(credit_note.id),
                reason: None,
            },
        )
        .await
        .expect("the refund is in flight");
    let refund_id = refund.provider_transaction_id.clone().expect("refund id");

    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_lf_inflight","kind":"payment_late_failure","external_id":"{external_id}"}}"#
        )
        .as_bytes(),
    )
    .await;
    let reversed = payment(&env, tx.id).await;
    assert_eq!(
        (reversed.amount_reversed, reversed.amount_reversed_excess),
        (tx.amount - 1000, 1000),
        "the refund in flight still holds 1000"
    );
    let due_before = env.get_invoices(sub_id).await[0].amount_due;

    let failed = format!(
        r#"{{"id":"evt_rf_failed","kind":"payment_refunded","external_id":"{external_id}","amount":0,"currency":"EUR","refunds":[{{"id":"{refund_id}","amount":1000,"succeeded":false,"pending":false}}]}}"#
    );
    for _ in 0..2 {
        env.deliver_mock_webhook(failed.as_bytes()).await;
        let applied = payment(&env, tx.id).await;
        assert_eq!(
            (applied.amount_reversed, applied.amount_reversed_excess),
            (tx.amount, 0),
            "the failed refund frees the 1000 the reversal could not apply"
        );
        assert_eq!(
            env.get_invoices(sub_id).await[0].amount_due,
            due_before + 1000,
            "the invoice owes exactly what was freed"
        );
    }
}

/// A refused refund surfaces the refusal, records it on the row, and gives the draw back: the
/// credit note can be refunded again.
#[rstest]
#[tokio::test]
async fn test_failed_online_refund_frees_the_credit_note_draw(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;

    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;
    env.set_mock_refund_behavior(MockRefundBehavior::Failed)
        .await;

    let command = RefundCommand {
        parent_transaction_id: tx.id,
        amount: 1000,
        credit_note_id: Some(credit_note.id),
        reason: None,
    };
    let err = env
        .services()
        .refund_transaction(actor(), TENANT_ID, command.clone())
        .await
        .expect_err("a refused refund must not read as an accepted one");
    assert!(matches!(err.current_context(), StoreError::PaymentError(_)));

    let refunds = refunds_of(&env, tx.id).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, PaymentStatusEnum::Failed);
    assert!(refunds[0].error_type.is_some());
    assert_eq!(
        payment(&env, tx.id).await.amount_refunded,
        0,
        "no money left the merchant"
    );

    let summary = env
        .store()
        .get_credit_note_refunds(TENANT_ID, credit_note.id)
        .await
        .expect("credit note refund summary");
    assert_eq!(
        summary.refundable_cents, 1000,
        "a failed attempt must not consume the avoir"
    );

    // Retrying the very same draw now goes through.
    env.set_mock_refund_behavior(MockRefundBehavior::Succeeded)
        .await;
    let refund = env
        .services()
        .refund_transaction(actor(), TENANT_ID, command)
        .await
        .expect("the retried refund is accepted");
    assert_eq!(refund.status, PaymentStatusEnum::Settled);
    assert_eq!(payment(&env, tx.id).await.amount_refunded, 1000);
}

// =============================================================================
// Offline refunds
// =============================================================================

/// A wire refund on a manually recorded payment: no provider is involved, so the row is written
/// settled with the date and reference the user gave.
#[rstest]
#[tokio::test]
async fn test_offline_refund_on_a_manual_payment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, parent) = manually_paid_invoice(&env).await;
    let credit_note = refund_credit_note(&env, invoice_id, 1200).await;

    let refund_date = NaiveDate::from_ymd_opt(2024, 2, 5)
        .unwrap()
        .and_hms_opt(9, 30, 0)
        .unwrap();

    let refund = env
        .services()
        .record_offline_refund(
            actor(),
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: parent.id,
                amount: 1200,
                credit_note_id: Some(credit_note.id),
                refund_date,
                reference: Some("wire-out-1".to_string()),
                reason: None,
            },
        )
        .await
        .expect("offline refund recorded");

    assert_eq!(refund.status, PaymentStatusEnum::Settled);
    assert_eq!(refund.refund_mode, Some(RefundMode::Offline));
    assert_eq!(refund.processed_at, Some(refund_date));
    assert_eq!(
        refund.provider_transaction_id.as_deref(),
        Some("wire-out-1")
    );
    assert_eq!(refund.credit_note_id, Some(credit_note.id));
    assert_eq!(payment(&env, parent.id).await.amount_refunded, 1200);

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
}

/// The same payment has no provider reference, so the online action must refuse it rather than
/// fail deep in a connector.
#[rstest]
#[tokio::test]
async fn test_online_refund_of_a_manual_payment_is_refused(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, parent) = manually_paid_invoice(&env).await;
    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;

    let err = env
        .services()
        .refund_transaction(
            actor(),
            TENANT_ID,
            RefundCommand {
                parent_transaction_id: parent.id,
                amount: 1000,
                credit_note_id: Some(credit_note.id),
                reason: None,
            },
        )
        .await
        .expect_err("a manually recorded payment cannot be refunded online");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));
    assert!(
        refunds_of(&env, parent.id).await.is_empty(),
        "the refused refund leaves no row behind"
    );
}

// =============================================================================
// What a refund may be drawn on
// =============================================================================

/// Without an avoir there is nothing to refund against — unless money was collected above the
/// invoice total.
#[rstest]
#[tokio::test]
async fn test_refund_without_a_credit_note_is_refused(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, _invoice_id, tx, _total) = env.settled_paid_card_invoice().await;

    let err = env
        .services()
        .refund_transaction(
            actor(),
            TENANT_ID,
            RefundCommand {
                parent_transaction_id: tx.id,
                amount: 500,
                credit_note_id: None,
                reason: None,
            },
        )
        .await
        .expect_err("a refund needs a finalized refund credit note");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));
}

/// A duplicate charge ("trop-perçu") is the one refund that needs no avoir, bounded by what was
/// collected above the invoice total.
#[rstest]
#[tokio::test]
async fn test_overpayment_is_refundable_without_a_credit_note(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;

    // A second charge the provider took on an already paid invoice: no supported flow produces
    // that state, so the precondition is written directly.
    {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRowNew {
            id: PaymentTransactionId::new(),
            tenant_id: TENANT_ID,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some("pi_duplicate".to_string()),
            amount: 1000,
            currency: "EUR".to_string(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: diesel_models::enums::PaymentStatusEnum::Settled,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: Some(now()),
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
        .insert(&mut conn)
        .await
        .expect("insert the duplicate charge");
    }

    // The original charge has 3500 left to give back, but only the 1000 collected above the
    // invoice total may go back without an avoir.
    let offline = |amount: i64| OfflineRefundCommand {
        parent_transaction_id: tx.id,
        amount,
        credit_note_id: None,
        refund_date: now(),
        reference: None,
        reason: None,
    };

    let err = env
        .services()
        .record_offline_refund(actor(), TENANT_ID, offline(1001))
        .await
        .expect_err("only the excess over the invoice total may be refunded without an avoir");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));

    let refund = env
        .services()
        .record_offline_refund(actor(), TENANT_ID, offline(1000))
        .await
        .expect("the overpayment itself is refundable");
    assert_eq!(refund.status, PaymentStatusEnum::Settled);
    assert!(refund.credit_note_id.is_none());
    assert_eq!(payment(&env, tx.id).await.amount_refunded, 1000);
}

/// A credit note may only be drawn once, up to what it credits.
#[rstest]
#[tokio::test]
async fn test_refund_cannot_exceed_its_credit_note(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;

    let command = |amount: i64| RefundCommand {
        parent_transaction_id: tx.id,
        amount,
        credit_note_id: Some(credit_note.id),
        reason: None,
    };

    let err = env
        .services()
        .refund_transaction(actor(), TENANT_ID, command(1001))
        .await
        .expect_err("a refund cannot exceed what the credit note allows");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));

    env.services()
        .refund_transaction(actor(), TENANT_ID, command(1000))
        .await
        .expect("the full draw is allowed");

    let err = env
        .services()
        .refund_transaction(actor(), TENANT_ID, command(1))
        .await
        .expect_err("an exhausted credit note allows nothing more");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));
    assert_eq!(refunds_of(&env, tx.id).await.len(), 1);
}

/// Voiding an avoir money was already given back on would leave the refund unbacked.
#[rstest]
#[tokio::test]
async fn test_voiding_a_refunded_credit_note_is_refused(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let credit_note = refund_credit_note(&env, invoice_id, 1000).await;

    env.services()
        .refund_transaction(
            actor(),
            TENANT_ID,
            RefundCommand {
                parent_transaction_id: tx.id,
                amount: 1000,
                credit_note_id: Some(credit_note.id),
                reason: None,
            },
        )
        .await
        .expect("refund settled");

    let err = env
        .store()
        .void_credit_note(actor(), TENANT_ID, credit_note.id)
        .await
        .expect_err("a credit note with a settled refund cannot be voided");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));
    assert_eq!(
        env.store()
            .get_credit_note_by_id(TENANT_ID, credit_note.id)
            .await
            .expect("get credit note")
            .status,
        CreditNoteStatus::Finalized
    );
}

// =============================================================================
// Manual reversal: the money never really landed
// =============================================================================

/// A wire that bounced is not a refund: the parent is clawed back and the invoice reopens for
/// its full total.
#[rstest]
#[tokio::test]
async fn test_reverse_manual_payment_reopens_the_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, _invoice_id, parent) = manually_paid_invoice(&env).await;
    let total = env.get_invoices(sub_id).await[0].total;

    let reversed = env
        .services()
        .reverse_manual_payment(
            actor(),
            TENANT_ID,
            ReverseManualPaymentCommand {
                transaction_id: parent.id,
                kind: ReversalKind::Returned,
                reason: Some("the wire bounced".to_string()),
                reversed_at: now(),
            },
        )
        .await
        .expect("the manual payment is reversed");

    assert_eq!(reversed.status, PaymentStatusEnum::Refunded);
    assert_eq!(reversed.amount_reversed, reversed.amount);
    assert_eq!(reversed.amount_disputed, 0);
    assert_eq!(reversed.amount_refunded, 0);
    assert_eq!(reversed.reversal_kind, Some(ReversalKind::Returned));
    assert_eq!(
        reversed.reversal_reason.as_deref(),
        Some("the wire bounced")
    );
    assert!(
        refunds_of(&env, parent.id).await.is_empty(),
        "an involuntary reversal writes no refund row"
    );

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Unpaid)
        .has_amount_due(total);
}

/// A payer recalling the wire is a claim against the merchant, like a chargeback: the money is
/// lost, the invoice stays paid and records the dispute.
#[rstest]
#[tokio::test]
async fn test_reverse_manual_payment_recalled_by_the_payer_keeps_the_invoice_paid(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, _invoice_id, parent) = manually_paid_invoice(&env).await;

    let reversed = env
        .services()
        .reverse_manual_payment(
            actor(),
            TENANT_ID,
            ReverseManualPaymentCommand {
                transaction_id: parent.id,
                kind: ReversalKind::DebtorRecall,
                reason: Some("payer recalled the wire".to_string()),
                reversed_at: now(),
            },
        )
        .await
        .expect("the manual payment is reversed");

    assert_eq!(reversed.amount_reversed, reversed.amount);
    assert_eq!(reversed.amount_disputed, reversed.amount);
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Paid)
        .has_amount_due(0);
    assert!(invoices[0].payment_disputed_at.is_some());
}

/// A provider-backed payment is clawed back by the provider's own event; reversing it by hand
/// would desynchronize us from the gateway.
#[rstest]
#[tokio::test]
async fn test_reverse_manual_payment_refuses_a_provider_payment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, _invoice_id, tx, _total) = env.settled_paid_card_invoice().await;

    let err = env
        .services()
        .reverse_manual_payment(
            actor(),
            TENANT_ID,
            ReverseManualPaymentCommand {
                transaction_id: tx.id,
                kind: ReversalKind::Chargeback,
                reason: None,
                reversed_at: now(),
            },
        )
        .await
        .expect_err("only a manually recorded payment can be reversed by hand");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));
    assert_eq!(payment(&env, tx.id).await.amount_reversed, 0);
}

// =============================================================================
// Refunds the merchant issued from the provider's dashboard
// =============================================================================

/// A refund we never issued is recorded from the provider's cumulative figure, linked to no
/// credit note, and leaves the invoice paid. A redelivered event changes nothing.
#[rstest]
#[tokio::test]
async fn test_observed_dashboard_refund_keeps_the_invoice_paid_and_is_idempotent(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, _invoice_id, tx, _total) = env.settled_paid_card_invoice().await;
    let external_id = tx.provider_transaction_id.clone().unwrap();

    let payload = refunded_payload(&external_id, 1500, "re_dashboard_1");
    env.deliver_mock_webhook(&payload).await;
    env.deliver_mock_webhook(&payload).await;

    let refunds = refunds_of(&env, tx.id).await;
    assert_eq!(refunds.len(), 1, "a redelivered refund event is a no-op");
    assert_eq!(refunds[0].amount, 1500);
    assert_eq!(refunds[0].status, PaymentStatusEnum::Settled);
    assert_eq!(refunds[0].refund_mode, Some(RefundMode::Online));
    assert!(
        refunds[0].credit_note_id.is_none(),
        "a dashboard refund is drawn on no avoir"
    );
    assert_eq!(
        refunds[0].provider_transaction_id.as_deref(),
        Some("re_dashboard_1")
    );

    let parent = payment(&env, tx.id).await;
    assert_eq!(parent.status, PaymentStatusEnum::Settled);
    assert_eq!(parent.amount_refunded, 1500);
    assert_eq!(parent.amount_reversed, 0);

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
}

// =============================================================================
// Regressions: the two sides of the balance must move in step, and the
// provider's cumulative total only ever speaks for the provider's own refunds.
// =============================================================================

/// An invoice paid in part by wire, so the avoir and the refund below land on a balance that is
/// still outstanding — where crediting and refunding the same money can drift apart.
async fn partially_paid_invoice(
    env: &TestEnv,
) -> (SubscriptionId, InvoiceId, PaymentTransaction, i64) {
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    let total = env.get_detailed_invoice(invoice_id).await.invoice.total;

    let tx = env
        .services()
        .add_manual_payment_transaction(
            actor(),
            TENANT_ID,
            invoice_id,
            dec!(14),
            now().and_utc().fixed_offset(),
            Some("wire-part-1".to_string()),
        )
        .await
        .expect("partial manual payment recorded");
    env.run_outbox_and_orchestration().await;

    (sub_id, invoice_id, tx, total)
}

/// Crediting 1400 of a partly paid invoice and handing that same 1400 back must leave the
/// customer owing the uncredited remainder — not the remainder minus money they no longer have.
#[rstest]
#[tokio::test]
async fn test_refund_on_a_partially_paid_invoice_restores_what_is_owed(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id, parent, total) = partially_paid_invoice(&env).await;

    let credit_note = refund_credit_note(&env, invoice_id, 1400).await;
    assert_eq!(
        env.get_invoices(sub_id).await[0].amount_due,
        total - 1400 - 1400,
        "the avoir reduces the invoice while the payment still covers what it paid for"
    );

    env.services()
        .record_offline_refund(
            actor(),
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: parent.id,
                amount: 1400,
                credit_note_id: Some(credit_note.id),
                refund_date: now(),
                reference: Some("wire-out-part".to_string()),
                reason: None,
            },
        )
        .await
        .expect("offline refund recorded");

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices[0].amount_due,
        total - 1400,
        "the money went back, so only the avoir still reduces what is owed"
    );
    assert_eq!(
        invoices[0].payment_status,
        InvoicePaymentStatus::Unpaid,
        "nothing of what the invoice still collects has been paid"
    );
}

/// An offline refund is invisible to the provider, so it must never be counted against the
/// cumulative total the provider reports for its own refunds.
#[rstest]
#[tokio::test]
async fn test_offline_refund_does_not_absorb_a_later_provider_refund(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, parent, _total) = env.settled_paid_card_invoice().await;
    let external_id = parent
        .provider_transaction_id
        .clone()
        .expect("a settled charge records a provider id");

    let credit_note = refund_credit_note(&env, invoice_id, 500).await;
    env.services()
        .record_offline_refund(
            actor(),
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: parent.id,
                amount: 500,
                credit_note_id: Some(credit_note.id),
                refund_date: now(),
                reference: Some("wire-out-2".to_string()),
                reason: None,
            },
        )
        .await
        .expect("offline refund recorded");

    env.deliver_mock_webhook(&refunded_payload(&external_id, 1000, "mock_re_dash"))
        .await;

    let refunds = refunds_of(&env, parent.id).await;
    assert_eq!(refunds.len(), 2, "the two refunds are unrelated");
    let observed = refunds
        .iter()
        .find(|r| r.refund_mode == Some(RefundMode::Online))
        .expect("the provider-side refund was recorded");
    assert_eq!(
        observed.amount, 1000,
        "the provider's cumulative figure covers only its own refunds"
    );
    assert_eq!(
        payment(&env, parent.id).await.amount_refunded,
        1500,
        "the payment carries both refunds"
    );
}

/// One event naming several refunds we have no row for: each gets its own row, keyed by the
/// provider's refund id so a later status change can still find it.
#[rstest]
#[tokio::test]
async fn test_several_observed_refunds_are_recorded_separately(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, _invoice_id, parent, _total) = env.settled_paid_card_invoice().await;
    let external_id = parent
        .provider_transaction_id
        .clone()
        .expect("a settled charge records a provider id");

    let payload = format!(
        r#"{{"id":"evt_re_multi","kind":"payment_refunded","external_id":"{external_id}","amount":1000,"currency":"EUR","refunds":[{{"id":"mock_re_a","amount":400,"succeeded":true}},{{"id":"mock_re_b","amount":600,"succeeded":true}}]}}"#
    );
    env.deliver_mock_webhook(payload.as_bytes()).await;

    let refunds = refunds_of(&env, parent.id).await;
    assert_eq!(refunds.len(), 2, "one row per refund the provider named");
    let mut named: Vec<(Option<&str>, i64)> = refunds
        .iter()
        .map(|r| (r.provider_transaction_id.as_deref(), r.amount))
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![(Some("mock_re_a"), 400), (Some("mock_re_b"), 600)],
        "each row keeps the refund id it belongs to"
    );
    assert_eq!(payment(&env, parent.id).await.amount_refunded, 1000);
}

/// The reason a refund was issued is part of the record, not just an argument to the provider.
#[rstest]
#[tokio::test]
async fn test_offline_refund_keeps_its_reason(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, parent) = manually_paid_invoice(&env).await;
    let credit_note = refund_credit_note(&env, invoice_id, 900).await;

    let refund = env
        .services()
        .record_offline_refund(
            actor(),
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: parent.id,
                amount: 900,
                credit_note_id: Some(credit_note.id),
                refund_date: now(),
                reference: Some("wire-out-3".to_string()),
                reason: Some("Duplicate charge".to_string()),
            },
        )
        .await
        .expect("offline refund recorded");

    assert_eq!(
        refund.reversal_reason.as_deref(),
        Some("Duplicate charge"),
        "the reason the caller gave is stored on the refund"
    );
}

/// A partial claw-back leaves the payment `Settled`, which must not be mistaken for a fresh
/// settlement: re-running the settled orchestration would mark the reopened invoice paid again.
/// A bank return reported as a dispute is a failed collection, so the invoice reopens.
#[rstest]
#[tokio::test]
async fn test_partial_chargeback_does_not_resettle_the_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, _invoice_id, parent, _total) = env.settled_paid_card_invoice().await;
    let external_id = parent
        .provider_transaction_id
        .clone()
        .expect("a settled charge records a provider id");

    let occurred_at = chrono::Utc::now().to_rfc3339();
    let payload = format!(
        r#"{{"id":"evt_dw_partial","kind":"dispute_funds_withdrawn","external_id":"{external_id}","amount":900,"currency":"EUR","reason":"insufficient_funds","occurred_at":"{occurred_at}"}}"#
    );
    env.deliver_mock_webhook(payload.as_bytes()).await;

    let reversed = payment(&env, parent.id).await;
    assert_eq!(reversed.status, PaymentStatusEnum::Settled);
    assert_eq!(reversed.amount_reversed, 900);

    env.run_outbox_and_orchestration().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        invoices[0].payment_status,
        InvoicePaymentStatus::PartiallyPaid,
        "the clawed-back part is owed again and stays owed"
    );
    assert_eq!(invoices[0].amount_due, 900);
}

/// A refund the provider may or may not have taken: the call failed in transport, so the row
/// never left `Ready`. It holds the avoir's remainder and the payment's refundable amount, so
/// the sweep re-issues it on the same idempotency key rather than leaving it to be fixed by hand.
#[rstest]
#[tokio::test]
async fn test_unconfirmed_refund_is_replayed_and_settles(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (_sub_id, invoice_id, parent, _total) = env.settled_paid_card_invoice().await;
    let credit_note = refund_credit_note(&env, invoice_id, 700).await;

    let refund_id = PaymentTransactionId::new();
    {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRowNew {
            id: refund_id,
            tenant_id: TENANT_ID,
            invoice_id: Some(invoice_id),
            provider_transaction_id: None,
            amount: 700,
            currency: "EUR".to_string(),
            payment_method_id: parent.payment_method_id,
            status: diesel_models::enums::PaymentStatusEnum::Ready,
            payment_type: diesel_models::enums::PaymentTypeEnum::Refund,
            error_type: None,
            processed_at: None,
            checkout_session_id: None,
            pending_plan_version_id: None,
            next_action: None,
            initiated_by_customer_id: None,
            pending_provider_intent_id: None,
            pending_connection_id: None,
            parent_transaction_id: Some(parent.id),
            credit_note_id: Some(credit_note.id),
            refund_mode: Some(diesel_models::enums::RefundModeEnum::Online),
            reversal_reason: None,
        }
        .insert(&mut conn)
        .await
        .expect("insert the unconfirmed refund");
    }

    // Until it resolves, the credit note is fully drawn and nothing more may be taken from it.
    let err = env
        .services()
        .record_offline_refund(
            actor(),
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: parent.id,
                amount: 1,
                credit_note_id: Some(credit_note.id),
                refund_date: now(),
                reference: None,
                reason: None,
            },
        )
        .await
        .expect_err("an unresolved refund still reserves the credit note");
    assert!(matches!(
        err.current_context(),
        StoreError::InvalidArgument(_)
    ));

    env.services()
        .replay_unconfirmed_refund(refund_id, TENANT_ID)
        .await
        .expect("the refund is re-issued on the same idempotency key");

    let replayed = payment(&env, refund_id).await;
    assert_eq!(replayed.status, PaymentStatusEnum::Settled);
    assert!(
        replayed
            .provider_transaction_id
            .as_deref()
            .is_some_and(|id| id.starts_with("mock_re_")),
        "the provider's own refund id is recorded this time"
    );
    assert_eq!(payment(&env, parent.id).await.amount_refunded, 700);
}
