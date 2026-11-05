//! Reconciliation and uncertain-payment paths — what a provider sandbox has to
//! certify beyond capture and decline.
//!
//! Webhook delivery is best-effort, so a `Pending` transaction can be settled,
//! failed or forgotten at the provider while we still believe it is in flight.
//! The reconciliation sweep asks the provider directly; these tests drive each
//! answer it can give through the production consolidation pipeline.
//!
//! The uncertain-charge case is the other half: a transport failure is NOT a
//! decline — the money may well have moved — so it must never be recorded as
//! one, and the invoice must stay collectable.
//!
//! Settlement reaches the invoice through the outbox, so each test pumps it
//! exactly as the webhook suite does.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};

use common_domain::ids::{InvoiceId, PaymentTransactionId};
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::connectors::{MockChargeBehavior, MockReconcileStatus};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

/// A finalized, unpaid card invoice to charge in each test.
async fn unpaid_invoice(env: &TestEnv) -> InvoiceId {
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_unpaid();
    invoices[0].id
}

/// Charge asynchronously so the transaction stays `Pending` with a provider id —
/// the exact shape the reconciliation sweep picks up.
async fn pending_charge(env: &TestEnv, invoice_id: InvoiceId) -> (PaymentTransactionId, String) {
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;

    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("async charge accepted");

    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    let provider_id = tx
        .provider_transaction_id
        .clone()
        .expect("async charge backfills a provider id");
    (tx.id, provider_id)
}

async fn transaction_status(env: &TestEnv, provider_id: &str) -> PaymentStatusEnum {
    env.store()
        .get_payment_tx_by_provider_transaction_id(TENANT_ID, provider_id)
        .await
        .expect("lookup by provider id")
        .expect("transaction still present")
        .status
}

/// The lost-webhook case: the provider took the money, the event never arrived.
/// Reconciliation must settle the transaction and pay the invoice off.
#[rstest]
#[tokio::test]
async fn test_reconcile_settles_a_payment_whose_webhook_never_arrived(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (tx_id, provider_id) = pending_charge(&env, invoice_id).await;

    env.set_mock_reconcile_status(MockReconcileStatus::Succeeded)
        .await;
    env.services()
        .reconcile_pending_transaction(tx_id, TENANT_ID)
        .await
        .expect("reconcile settles the payment the provider confirms");
    env.run_outbox_and_orchestration().await;

    assert_eq!(
        transaction_status(&env, &provider_id).await,
        PaymentStatusEnum::Settled
    );

    let invoice = env.get_detailed_invoice(invoice_id).await;
    assert_eq!(
        invoice.invoice.amount_due, 0,
        "a reconciled settlement must pay the invoice off, not just flip the transaction"
    );
}

/// The provider refused the charge and the failure webhook was lost too.
#[rstest]
#[tokio::test]
async fn test_reconcile_marks_a_failed_charge_failed(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (tx_id, provider_id) = pending_charge(&env, invoice_id).await;

    env.set_mock_reconcile_status(MockReconcileStatus::Failed)
        .await;
    env.services()
        .reconcile_pending_transaction(tx_id, TENANT_ID)
        .await
        .expect("reconcile records the provider's failure");
    env.run_outbox_and_orchestration().await;

    assert_eq!(
        transaction_status(&env, &provider_id).await,
        PaymentStatusEnum::Failed
    );

    let invoice = env.get_detailed_invoice(invoice_id).await;
    assert!(
        invoice.invoice.amount_due > 0,
        "a failed charge leaves the invoice collectable"
    );
}

/// The provider has no record of an id WE stored — almost always an account or
/// environment mismatch, not a lost payment. Cancelling would be terminal and
/// would make a later genuine settlement webhook a no-op, so the row must stay
/// Pending for a human to look at.
#[rstest]
#[tokio::test]
async fn test_reconcile_never_cancels_a_transaction_the_provider_cannot_find(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (tx_id, provider_id) = pending_charge(&env, invoice_id).await;

    env.set_mock_reconcile_status(MockReconcileStatus::Unknown)
        .await;
    env.services()
        .reconcile_pending_transaction(tx_id, TENANT_ID)
        .await
        .expect("an unknown transaction is not an error");
    env.run_outbox_and_orchestration().await;

    assert_eq!(
        transaction_status(&env, &provider_id).await,
        PaymentStatusEnum::Pending,
        "never auto-cancel on a 404: a later settlement webhook must still apply"
    );
}

/// A transport failure is an UNCERTAIN outcome, not a decline: the charge may
/// have taken the money. It must not be recorded as a failed payment, and the
/// invoice must stay collectable so a retry can go through.
#[rstest]
#[tokio::test]
async fn test_uncertain_charge_is_not_recorded_as_a_decline(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;
    let err = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect_err("an uncertain charge must not resolve to a payment outcome");
    log::info!("uncertain charge surfaced as: {err:?}");

    let transactions = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions");
    assert!(
        !transactions
            .iter()
            .any(|tx| tx.transaction.status == PaymentStatusEnum::Failed),
        "an uncertain charge is not a decline; recording one would send the \
         customer a false 'payment failed' and start dunning"
    );

    let invoice = env.get_detailed_invoice(invoice_id).await;
    assert!(invoice.invoice.amount_due > 0, "invoice stays collectable");

    // The attempt was committed before it was sent, so it stays in flight with no provider
    // answer: a second click is fenced instead of minting a new key (a double charge).
    let unsent = transactions
        .iter()
        .find(|tx| tx.transaction.status == PaymentStatusEnum::Pending)
        .expect("the uncertain attempt stays Pending")
        .transaction
        .clone();
    assert!(unsent.provider_transaction_id.is_none());
    let err = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect_err("a second attempt waits for the first one's outcome");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::PaymentError(_)
    ));

    // The sweep re-sends it under its committed key, and the provider's answer settles it.
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.services()
        .resume_unsent_invoice_charge(unsent.id, TENANT_ID)
        .await
        .expect("resume the unsent charge");
    env.run_outbox_and_orchestration().await;

    let transactions = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions");
    assert_eq!(transactions.len(), 1, "the re-send is the same attempt");
    assert_eq!(
        transactions[0].transaction.status,
        PaymentStatusEnum::Settled
    );

    let invoice = env.get_detailed_invoice(invoice_id).await;
    assert_eq!(
        invoice.invoice.amount_due, 0,
        "the resumed charge pays the invoice off"
    );
}

/// Past the provider's idempotency window a re-send could charge twice, so an attempt that
/// never got an answer is failed instead, which frees the invoice for its next attempt.
#[rstest]
#[tokio::test]
async fn test_unsent_charge_past_its_replay_window_is_failed(#[future] test_env: TestEnv) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::payment_transaction::dsl as pt;

    let env = test_env.await;
    env.seed_payments().await;
    let invoice_id = unpaid_invoice(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;
    let err = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect_err("uncertain charge");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::PaymentProviderError
    ));

    let unsent = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions")
        .remove(0)
        .transaction;

    let mut conn = env.conn().await;
    diesel::update(pt::payment_transaction.filter(pt::id.eq(unsent.id)))
        .set(pt::created_at.eq(chrono::Utc::now() - chrono::Duration::days(2)))
        .execute(&mut conn)
        .await
        .expect("age the attempt");
    drop(conn);

    let claimed = env
        .services()
        .claim_unsent_invoice_charges(chrono::Utc::now(), 10)
        .await
        .expect("claim unsent charges");
    assert!(claimed.iter().any(|r| r.id == unsent.id));

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.services()
        .resume_unsent_invoice_charge(unsent.id, TENANT_ID)
        .await
        .expect("resume");

    let tx = env
        .store()
        .get_payment_tx_by_id(TENANT_ID, unsent.id)
        .await
        .expect("transaction");
    assert_eq!(
        tx.status,
        PaymentStatusEnum::Failed,
        "never re-sent once the provider may have forgotten the key"
    );
    assert!(tx.provider_transaction_id.is_none());
}

/// An uncertain charge whose invoice was paid by other means (or voided) meanwhile is not
/// re-sent: the attempt is failed as unconfirmed, without dunning.
#[rstest]
#[tokio::test]
async fn test_unsent_charge_is_not_resent_once_the_invoice_is_settled(#[future] test_env: TestEnv) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::invoice::dsl as inv;

    let env = test_env.await;
    env.seed_payments().await;
    let invoice_id = unpaid_invoice(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;
    let _ = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await;
    let unsent = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions")
        .remove(0)
        .transaction;

    // Settled out of band (bank transfer recorded by an operator).
    let mut conn = env.conn().await;
    diesel::update(inv::invoice.filter(inv::id.eq(invoice_id)))
        .set(inv::amount_due.eq(0))
        .execute(&mut conn)
        .await
        .expect("settle invoice out of band");
    drop(conn);

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.services()
        .resume_unsent_invoice_charge(unsent.id, TENANT_ID)
        .await
        .expect("resume");

    let tx = env
        .store()
        .get_payment_tx_by_id(TENANT_ID, unsent.id)
        .await
        .expect("transaction");
    assert_eq!(tx.status, PaymentStatusEnum::Failed, "never re-sent");
    assert_eq!(
        tx.decline_kind,
        Some(meteroid_store::domain::DeclineKind::Unconfirmed)
    );
}

/// A re-send the provider refuses says nothing about the first send, which may have charged:
/// the attempt is kept (failed as unconfirmed), never deleted.
#[rstest]
#[tokio::test]
async fn test_refused_resend_keeps_the_attempt(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    let invoice_id = unpaid_invoice(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Unknown)
        .await;
    let _ = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await;
    let unsent = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions")
        .remove(0)
        .transaction;

    env.set_mock_charge_behavior(MockChargeBehavior::RejectedCredentials)
        .await;
    env.services()
        .resume_unsent_invoice_charge(unsent.id, TENANT_ID)
        .await
        .expect("resume");

    let tx = env
        .store()
        .get_payment_tx_by_id(TENANT_ID, unsent.id)
        .await
        .expect("the attempt is kept");
    assert_eq!(tx.status, PaymentStatusEnum::Failed);
    assert_eq!(
        tx.decline_kind,
        Some(meteroid_store::domain::DeclineKind::Unconfirmed)
    );
}

/// A renewal-style charge that needs 3DS: nobody is present, so it is left awaiting the
/// customer's authentication.
async fn charge_awaiting_authentication(
    env: &TestEnv,
    invoice_id: InvoiceId,
) -> (PaymentTransactionId, String) {
    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;

    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            false,
            None,
        )
        .await
        .expect("charge accepted, awaiting authentication");

    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    let provider_id = tx
        .provider_transaction_id
        .clone()
        .expect("the challenged charge carries a provider id");
    (tx.id, provider_id)
}

async fn awaiting_authentication(env: &TestEnv, invoice_id: InvoiceId) -> Vec<bool> {
    env.store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list invoice transactions")
        .into_iter()
        .map(|tx| tx.awaiting_authentication)
        .collect()
}

/// The customer comes back to an invoice whose renewal charge needs 3DS: the challenge is re-read
/// from the provider with its secret (never stored), and answering it pays the invoice.
#[rstest]
#[tokio::test]
async fn test_resume_authentication_returns_the_challenge_then_settles(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::payment_transactions::PaymentNextAction;

    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (tx_id, provider_id) = charge_awaiting_authentication(&env, invoice_id).await;
    assert_eq!(awaiting_authentication(&env, invoice_id).await, vec![true]);

    let action = env
        .services()
        .resume_invoice_payment_authentication(TENANT_ID, invoice_id)
        .await
        .expect("the awaiting charge can be resumed");
    match action {
        PaymentNextAction::UseSdk {
            intent_id,
            client_secret,
            ..
        } => {
            assert_eq!(intent_id, provider_id);
            assert!(
                client_secret.is_some(),
                "a resumed action must carry the secret the stored one never holds"
            );
        }
        other => panic!("expected an SDK action, got {other:?}"),
    }

    // The customer approved: the provider now reports the charge as succeeded.
    env.set_mock_reconcile_status(MockReconcileStatus::Succeeded)
        .await;
    env.services()
        .reconcile_pending_transaction(tx_id, TENANT_ID)
        .await
        .expect("reconcile settles the authenticated payment");
    env.run_outbox_and_orchestration().await;

    assert_eq!(
        transaction_status(&env, &provider_id).await,
        PaymentStatusEnum::Settled
    );
    assert_eq!(awaiting_authentication(&env, invoice_id).await, vec![false]);
    assert_eq!(
        env.get_detailed_invoice(invoice_id)
            .await
            .invoice
            .amount_due,
        0
    );
}

/// A plain in-flight payment (async debit) waits on the provider, not the customer: there is
/// nothing to resume and no secret to hand out.
#[rstest]
#[tokio::test]
async fn test_resume_authentication_refuses_a_payment_not_awaiting_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    pending_charge(&env, invoice_id).await;
    assert_eq!(awaiting_authentication(&env, invoice_id).await, vec![false]);

    let err = env
        .services()
        .resume_invoice_payment_authentication(TENANT_ID, invoice_id)
        .await
        .expect_err("nothing awaits authentication");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::FailedPrecondition(_)
    ));
}

async fn assert_resume_refused(env: &TestEnv, invoice_id: InvoiceId) {
    let err = env
        .services()
        .resume_invoice_payment_authentication(TENANT_ID, invoice_id)
        .await
        .expect_err("the payment must not be resumable");
    assert!(matches!(
        err.current_context(),
        meteroid_store::errors::StoreError::FailedPrecondition(_)
    ));
}

/// Handing out the secret is what lets the money move: a charge left behind on an invoice that
/// was voided since must not be completable.
#[rstest]
#[tokio::test]
async fn test_resume_authentication_refuses_a_voided_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    charge_awaiting_authentication(&env, invoice_id).await;
    env.try_void_invoice(invoice_id)
        .await
        .expect("invoice voided");

    assert_resume_refused(&env, invoice_id).await;
}

/// The challenge lapsed or was refused at the bank: the attempt ends as failed, so the invoice is
/// collectable again instead of staying blocked behind a pending row.
#[rstest]
#[tokio::test]
async fn test_resume_authentication_fails_a_lapsed_challenge(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (_, provider_id) = charge_awaiting_authentication(&env, invoice_id).await;
    let amount_due = env
        .get_detailed_invoice(invoice_id)
        .await
        .invoice
        .amount_due;

    env.set_mock_reconcile_status(MockReconcileStatus::Failed)
        .await;
    assert_resume_refused(&env, invoice_id).await;
    env.run_outbox_and_orchestration().await;

    assert_eq!(
        transaction_status(&env, &provider_id).await,
        PaymentStatusEnum::Failed
    );
    assert_eq!(awaiting_authentication(&env, invoice_id).await, vec![false]);
    assert_eq!(
        env.get_detailed_invoice(invoice_id)
            .await
            .invoice
            .amount_due,
        amount_due
    );
}

/// Authenticated and now settling at the provider: still pending, but no longer waiting on the
/// customer, so the challenge stops being offered.
#[rstest]
#[tokio::test]
async fn test_resume_authentication_stops_offering_a_settling_payment(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (_, provider_id) = charge_awaiting_authentication(&env, invoice_id).await;

    env.set_mock_reconcile_status(MockReconcileStatus::Pending)
        .await;
    assert_resume_refused(&env, invoice_id).await;

    assert_eq!(
        transaction_status(&env, &provider_id).await,
        PaymentStatusEnum::Pending
    );
    assert_eq!(awaiting_authentication(&env, invoice_id).await, vec![false]);
}

/// A debit the provider accepted marks the invoice Processing, the way billing does. If the
/// provider then cancels it (a Stancer batch capture that never ran), nothing is in flight any
/// more: the invoice must be payable again, not left Processing and fenced off for good.
#[rstest]
#[tokio::test]
async fn test_cancelled_in_flight_debit_reopens_the_processing_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_uber_sepa_payment_method().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    let invoice_id = env.get_invoices(sub_id).await[0].id;

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment(TENANT_ID, invoice_id, CUST_UBER_SEPA_METHOD_ID, true, None)
        .await
        .expect("debit accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_processing();

    env.set_mock_reconcile_status(MockReconcileStatus::Cancelled)
        .await;
    env.services()
        .reconcile_pending_transaction(tx.id, TENANT_ID)
        .await
        .expect("reconcile records the cancellation");
    env.run_outbox_and_orchestration().await;

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_unpaid();
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        0,
        "a cancellation is not a decline: no dunning"
    );
}

/// Backdate a pending transaction and its last poll, the state the sweep sees a day later.
async fn age_transaction(env: &TestEnv, tx_id: PaymentTransactionId, age: &str, last_poll: &str) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query(format!(
        "UPDATE payment_transaction SET created_at = NOW() - INTERVAL '{age}', \
         last_reconciled_at = NOW() - INTERVAL '{last_poll}' WHERE id = $1"
    ))
    .bind::<diesel::sql_types::Uuid, _>(*tx_id)
    .execute(&mut conn)
    .await
    .expect("failed to backdate transaction");
}

async fn claimed(env: &TestEnv, tx_id: PaymentTransactionId) -> bool {
    env.services()
        .claim_pending_payment_transactions(chrono::Utc::now() + chrono::Duration::minutes(1), 50)
        .await
        .expect("claim pending transactions")
        .iter()
        .any(|r| r.id == tx_id)
}

/// A charge that only settles by polling (a Stancer batch capture) must not be polled every
/// sweep: once claimed it waits a tenth of its age, capped at an hour, before the next poll.
#[rstest]
#[tokio::test]
async fn test_reconcile_backs_off_a_long_pending_charge(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let invoice_id = unpaid_invoice(&env).await;
    let (tx_id, _) = pending_charge(&env, invoice_id).await;

    assert!(claimed(&env, tx_id).await, "first poll");
    assert!(!claimed(&env, tx_id).await, "just polled");

    age_transaction(&env, tx_id, "1 day", "30 minutes").await;
    assert!(
        !claimed(&env, tx_id).await,
        "an hour between polls at a day old"
    );

    age_transaction(&env, tx_id, "1 day", "61 minutes").await;
    assert!(claimed(&env, tx_id).await, "due again after the hour");
}
