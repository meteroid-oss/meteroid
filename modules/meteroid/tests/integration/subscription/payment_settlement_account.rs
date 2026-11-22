//! Every payment path settles an issued invoice from its account (`billing_core::settlement`):
//! what it still owes and its payment status are a fold of what moved its money, so a reversal,
//! an accepted debit or a redelivered settlement reads the same figures whichever path ran.

use chrono::NaiveDate;
use rstest::rstest;

use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId};
use diesel_models::payments::PaymentTransactionRow;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::enums::InvoicePaymentStatus;
use meteroid_store::domain::outbox_event::PaymentTransactionEvent;
use meteroid_store::domain::payment_transactions::PaymentTransaction;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};
use crate::helpers::credit::assert_ledger_balances;
use crate::helpers::invariants::assert_invoice_accounts;

async fn payments_of(env: &TestEnv, invoice_id: InvoiceId) -> Vec<PaymentTransaction> {
    env.store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions")
        .into_iter()
        .map(|t| t.transaction)
        .collect()
}

/// GoCardless-style `late_failure_settled`: the debit failed after it settled.
fn late_failure_payload(external_id: &str) -> Vec<u8> {
    format!(r#"{{"id":"evt_lf","kind":"payment_late_failure","external_id":"{external_id}"}}"#)
        .into_bytes()
}

fn succeeded_payload(tx_id: PaymentTransactionId, external_id: &str) -> Vec<u8> {
    format!(
        r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{external_id}","amount":3500,"currency":"EUR"}}"#,
        tx = tx_id.as_base62(),
    )
    .into_bytes()
}

/// A 3500 card invoice paying `credit` from the balance and the rest by a settled card charge,
/// its settlement handled: `(sub_id, invoice_id, the settled charge)`.
async fn credited_paid_invoice(
    env: &TestEnv,
    credit: i64,
) -> (
    common_domain::ids::SubscriptionId,
    InvoiceId,
    PaymentTransaction,
) {
    env.fund_invoice_credit(CUST_UBER_ID, credit).await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
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
        .expect("charge accepted");
    assert_eq!(tx.status, PaymentStatusEnum::Settled);
    assert_eq!(tx.amount, 3_500 - credit);
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_applied_credits(credit)
        .has_amount_due(0);
    (sub_id, invoice_id, tx)
}

/// 3500 billed, 1750 paid by credit, 1750 by a debit that failed after it settled: 1750 is owed
/// again and the invoice is Unpaid, the credit paying nothing in cash. Every path reads it so:
/// the webhook reversal, the settlement event redelivered after it, the reversal orchestration.
/// Measured against the total, the last two relabeled it PartiallyPaid.
/// Rules: R-125, R-060
#[rstest]
#[tokio::test]
async fn test_a_reversal_on_a_credited_invoice_reads_unpaid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx) = credited_paid_invoice(&env, 1_750).await;
    let stale: PaymentTransactionEvent = tx.clone().into();
    let external_id = tx.provider_transaction_id.clone().expect("provider id");

    env.deliver_mock_webhook(&late_failure_payload(&external_id))
        .await;
    let reversed = payments_of(&env, invoice_id).await.remove(0);
    assert_eq!(
        (
            reversed.status.clone(),
            reversed.amount_reversed,
            reversed.amount_disputed
        ),
        (PaymentStatusEnum::Refunded, 1_750, 0)
    );
    let reopened = |invoices: Vec<meteroid_store::domain::Invoice>| {
        invoices
            .assert()
            .invoice_at(0)
            .has_payment_status(InvoicePaymentStatus::Unpaid)
            .has_total(3_500)
            .has_applied_credits(1_750)
            .has_amount_due(1_750);
        assert!(invoices[0].paid_at.is_none(), "it owes again");
    };
    reopened(env.get_invoices(sub_id).await);

    env.services()
        .on_payment_transaction_settled(stale)
        .await
        .expect("the redelivered settlement is handled");
    reopened(env.get_invoices(sub_id).await);

    env.services()
        .on_payment_transaction_reversed(reversed.into())
        .await
        .expect("the reversal orchestration is handled");
    reopened(env.get_invoices(sub_id).await);

    assert_eq!(env.invoice_credit_balance_minor(CUST_UBER_ID).await, 0);
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_invoice_accounts(env.store(), TENANT_ID).await;
}

/// A SEPA debit the provider accepted is recorded on its payment: the account reads the 3500
/// invoice Processing, still owing 3500, and refuses another charge while it is in flight. Its
/// settlement pays it.
/// Rules: R-125, R-127
#[rstest]
#[tokio::test]
async fn test_an_accepted_debit_reads_processing_and_is_not_charged_again(
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
    assert_eq!(
        (tx.status.clone(), tx.amount),
        (PaymentStatusEnum::Pending, 3_500)
    );
    {
        let mut conn = env.conn().await;
        let row = PaymentTransactionRow::get_by_id(&mut conn, tx.id, TENANT_ID)
            .await
            .expect("debit row");
        assert!(
            row.accepted_at.is_some(),
            "the acceptance is recorded on the debit"
        );
    }
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_processing()
        .has_amount_due(3_500);
    assert_invoice_accounts(env.store(), TENANT_ID).await;

    let err = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            true,
            None,
        )
        .await
        .expect_err("the accepted debit owns the invoice");
    assert!(
        format!("{err:?}").contains("already being processed"),
        "unexpected error: {err:?}"
    );
    assert!(
        env.store()
            .invoice_has_live_payment(TENANT_ID, invoice_id)
            .await
            .expect("live-payment check")
    );
    assert_eq!(
        payments_of(&env, invoice_id).await.len(),
        1,
        "no second charge"
    );

    let external_id = tx.provider_transaction_id.clone().expect("provider id");
    env.deliver_mock_webhook(&succeeded_payload(tx.id, &external_id))
        .await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_amount_due(0);
    let payments = payments_of(&env, invoice_id).await;
    assert_eq!(payments.len(), 1);
    assert_eq!(
        (payments[0].status.clone(), payments[0].amount),
        (PaymentStatusEnum::Settled, 3_500)
    );
    assert_invoice_accounts(env.store(), TENANT_ID).await;
}

/// 3500 billed, 1000 paid by credit and 2500 by card: the settlement redelivered twice folds the
/// same account and changes nothing, its payment date included.
/// Rules: R-125
#[rstest]
#[tokio::test]
async fn test_a_redelivered_settlement_changes_nothing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id, tx) = credited_paid_invoice(&env, 1_000).await;
    let paid_at = env.get_invoices(sub_id).await[0].paid_at;
    assert!(paid_at.is_some());
    let event: PaymentTransactionEvent = tx.into();

    for _ in 0..2 {
        env.services()
            .on_payment_transaction_settled(event.clone())
            .await
            .expect("redelivered settlement handled");
    }

    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_paid()
        .has_total(3_500)
        .has_applied_credits(1_000)
        .has_amount_due(0);
    assert_eq!(invoices[0].paid_at, paid_at, "paid when it was first paid");
    let payments = payments_of(&env, invoice_id).await;
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount, 2_500);
    assert_eq!(
        env.invoice_credit_balance_minor(CUST_UBER_ID).await,
        0,
        "nothing kept as overpayment"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    assert_invoice_accounts(env.store(), TENANT_ID).await;
}

/// A payment the provider captures on an invoice voided meanwhile (an attempt nothing here knew
/// was moving) pays nothing on the void: the invoice stays as voided, never paid, and the 3500
/// captured is kept as customer credit.
/// Rules: R-128, R-125, OQ12
#[rstest]
#[tokio::test]
async fn test_a_payment_landing_on_a_voided_invoice_is_kept_as_credit(#[future] test_env: TestEnv) {
    use meteroid_store::domain::enums::InvoiceStatusEnum;

    let env = test_env.await;
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;
    env.try_void_invoice(invoice_id)
        .await
        .expect("nothing is moving toward the invoice");

    let tx_id = PaymentTransactionId::new();
    {
        let mut conn = env.conn().await;
        diesel_models::payments::PaymentTransactionRowNew {
            id: tx_id,
            tenant_id: TENANT_ID,
            invoice_id: Some(invoice_id),
            provider_transaction_id: Some("tr_stray".to_string()),
            amount: 3_500,
            currency: "EUR".to_string(),
            payment_method_id: Some(CUST_UBER_PAYMENT_METHOD_ID),
            status: diesel_models::enums::PaymentStatusEnum::Pending,
            payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
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
        .insert(&mut conn)
        .await
        .expect("an attempt the void did not see");
    }
    env.deliver_mock_webhook(&succeeded_payload(tx_id, "tr_stray"))
        .await;
    env.run_outbox_and_orchestration().await;

    let invoice = env.get_invoices(sub_id).await.remove(0);
    assert_eq!(invoice.status, InvoiceStatusEnum::Void);
    assert_ne!(invoice.payment_status, InvoicePaymentStatus::Paid);
    assert!(invoice.paid_at.is_none(), "a void is never paid");
    assert_eq!(
        env.invoice_credit_balance_minor(CUST_UBER_ID).await,
        3_500,
        "the captured payment is the customer's credit"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}
