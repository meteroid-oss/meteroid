//! Purchased credit spent before its money came back: every notice moves the reported loss of its
//! invoice to the current one, so lost then recovered amounts net to the real loss, and a replay
//! reports nothing.
use crate::data::ids::{CUST_UBER_ID, TENANT_ID};
use crate::helpers::credit::{assert_ledger_balances, available};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_customer_credit::{draft, finalize, line, pay_amount};
use chrono::NaiveDateTime;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, InvoiceId, PaymentTransactionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::sql_types::{BigInt, Uuid as SqlUuid};
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::domain::enums::{InvoicePaymentStatus, ReversalKind};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::wallet_events::{
    CreditUnrecoveredCause, WalletCreditRecoveredEvent, WalletCreditUnrecoveredEvent,
};
use meteroid_store::domain::{Invoice, LineItem, LineItemKind, PaymentStatusEnum};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::payment_transactions::{
    PaymentTransactionInterface, ReversalAmount, TransactionReinstatement, TransactionReversal,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use meteroid_store::services::OfflineRefundCommand;
use rust_decimal::Decimal;
use std::sync::Arc;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await
}

async fn balance(setup: &MeteroidSetup) -> i64 {
    available(&setup.store, TENANT_ID, CUST_UBER_ID, None).await
}

fn credit_pack(cents: i64) -> LineItem {
    LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(cents),
        ..line("Credit pack", cents, false)
    }
}

/// Every unrecovered-credit notice of the invoice the webhook queue holds, in publish order.
async fn notices(
    setup: &MeteroidSetup,
    invoice: InvoiceId,
) -> TestResult<Vec<WalletCreditUnrecoveredEvent>> {
    Ok(setup
        .store
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .map_err(test_error)?
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditUnrecovered(event) if event.invoice_id == Some(invoice) => {
                Some(*event)
            }
            _ => None,
        })
        .collect())
}

/// Every recovered-credit notice of the invoice the webhook queue holds, in publish order.
async fn recoveries(
    setup: &MeteroidSetup,
    invoice: InvoiceId,
) -> TestResult<Vec<WalletCreditRecoveredEvent>> {
    Ok(setup
        .store
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .map_err(test_error)?
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditRecovered(event) if event.invoice_id == Some(invoice) => {
                Some(*event)
            }
            _ => None,
        })
        .collect())
}

/// `(cause, recovered, loss still reported)` of each recovery.
fn recovered(notices: &[WalletCreditRecoveredEvent]) -> Vec<(CreditUnrecoveredCause, i64, i64)> {
    notices
        .iter()
        .map(|notice| {
            (
                notice.cause,
                notice.recovered_minor,
                notice.unrecovered_total_minor,
            )
        })
        .collect()
}

fn totals(notices: &[WalletCreditUnrecoveredEvent]) -> Vec<i64> {
    notices
        .iter()
        .map(|notice| notice.unrecovered_total_minor)
        .collect()
}

async fn sweep(setup: &MeteroidSetup) -> TestResult<usize> {
    setup
        .store
        .reconcile_reversed_credit_purchases(100)
        .await
        .map_err(test_error)
}

async fn reinstate(
    setup: &MeteroidSetup,
    payment: PaymentTransactionId,
    amount: Option<i64>,
    reinstated_at: NaiveDateTime,
) -> TestResult {
    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, payment)
        .await
        .map_err(test_error)?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    setup
        .store
        .reinstate_transaction_tx(
            &mut conn,
            &Actor::System,
            transaction,
            TransactionReinstatement {
                external_transaction_id: "ch_partial".to_owned(),
                reinstated_amount_minor: amount,
                reason: "dispute won".to_owned(),
                reinstated_at,
                provider_ref: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(())
}

/// A paid 5000 purchase whose credit then pays `spent` of a new invoice billing `billed`.
async fn purchase_spent_on(
    setup: &MeteroidSetup,
    spent: i64,
    billed: Vec<LineItem>,
) -> TestResult<(Invoice, Invoice, PaymentTransactionId)> {
    let purchase = draft(setup, vec![credit_pack(5_000)]).await?;
    let purchase = finalize(setup, purchase.id).await?.invoice;
    pay_amount(setup, purchase.id, 5_000).await?;
    let spend = draft(setup, billed).await?;
    let spend = finalize(setup, spend.id).await?.invoice;
    assert_eq!(spend.applied_credits, spent);
    assert_eq!(balance(setup).await, 5_000 - spent);
    let payment = settled_payment(setup, purchase.id).await?;
    Ok((purchase, spend, payment))
}

fn figures(notices: &[WalletCreditUnrecoveredEvent]) -> Vec<(CreditUnrecoveredCause, i64, i64)> {
    notices
        .iter()
        .map(|notice| {
            (
                notice.cause,
                notice.taken_back_minor,
                notice.unrecovered_minor,
            )
        })
        .collect()
}

async fn settled_payment(
    setup: &MeteroidSetup,
    invoice: InvoiceId,
) -> TestResult<PaymentTransactionId> {
    Ok(setup
        .store
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice)
        .await
        .map_err(test_error)?
        .into_iter()
        .find(|tx| tx.transaction.status == PaymentStatusEnum::Settled)
        .ok_or("the manual payment settled")?
        .transaction
        .id)
}

/// The bank claws `amount` more of the payment back, as the provider's webhook records it.
async fn charge_back(
    setup: &MeteroidSetup,
    payment: PaymentTransactionId,
    amount: i64,
    reversed_at: NaiveDateTime,
) -> TestResult {
    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, payment)
        .await
        .map_err(test_error)?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    setup
        .store
        .reverse_transaction_tx(
            &mut conn,
            &Actor::System,
            transaction,
            TransactionReversal {
                external_transaction_id: "ch_partial".to_owned(),
                amount: ReversalAmount::Incremental(amount),
                kind: ReversalKind::Chargeback,
                reason: Some("dispute lost".to_owned()),
                reversed_at,
                disputed: true,
                provider_ref: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(())
}

/// The bank returns `amount` more of the payment after it settled: a failed collection, the
/// invoice reopens for it.
async fn fail_collection(
    setup: &MeteroidSetup,
    payment: PaymentTransactionId,
    amount: i64,
    reversed_at: NaiveDateTime,
) -> TestResult {
    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, payment)
        .await
        .map_err(test_error)?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    setup
        .store
        .reverse_transaction_tx(
            &mut conn,
            &Actor::System,
            transaction,
            TransactionReversal {
                external_transaction_id: "ch_returned".to_owned(),
                amount: ReversalAmount::Incremental(amount),
                kind: ReversalKind::InsufficientFunds,
                reason: Some("insufficient_funds".to_owned()),
                reversed_at,
                disputed: false,
                provider_ref: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(())
}

/// Spends `cents` of the wallet on a new invoice.
async fn spend(setup: &MeteroidSetup, cents: i64) -> TestResult {
    let spend = draft(setup, vec![line("Seats", cents, false)]).await?;
    assert_eq!(
        finalize(setup, spend.id).await?.invoice.applied_credits,
        cents
    );
    Ok(())
}

/// Two equal 1000 chargebacks of one payment of a spent purchase. Old code derived both notices'
/// id from (invoice, payment, 1000), so the webhook layer dropped the second as a duplicate and
/// the merchant learnt of 1000 lost instead of 2000.
#[tokio::test]
async fn equal_partial_chargebacks_of_one_payment_are_two_notices() -> TestResult {
    let setup = setup().await;
    let invoice = draft(&setup, vec![credit_pack(5_000)]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    pay_amount(&setup, invoice.id, 5_000).await?;
    spend(&setup, 5_000).await?;
    let payment = settled_payment(&setup, invoice.id).await?;

    let first_at = chrono::Utc::now().naive_utc();
    let second_at = first_at + chrono::Duration::seconds(1);
    charge_back(&setup, payment, 1_000, first_at).await?;
    charge_back(&setup, payment, 1_000, second_at).await?;

    let reported = notices(&setup, invoice.id).await?;
    assert_eq!(
        figures(&reported),
        vec![
            (CreditUnrecoveredCause::PaymentDisputed, 0, 1_000),
            (CreditUnrecoveredCause::PaymentDisputed, 0, 1_000),
        ]
    );
    assert_ne!(
        reported[0].id, reported[1].id,
        "distinct event ids, so both are delivered"
    );
    assert_eq!(totals(&reported), vec![1_000, 2_000]);

    // The provider redelivers the second chargeback: nothing more is clawed back or reported.
    charge_back(&setup, payment, 1_000, second_at).await?;
    let reported = notices(&setup, invoice.id).await?;
    assert_eq!(reported.len(), 2);
    assert_eq!(
        reported
            .iter()
            .map(|notice| notice.unrecovered_minor)
            .sum::<i64>(),
        2_000
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// 7000 invoice: 5000 of credit next to a 2000 support line refunded in cash, the credit all
/// spent, then the remaining 5000 charged back. All of it backed the credit; old code spread the
/// reversal over the whole 7000 and reported 3572.
#[tokio::test]
async fn a_chargeback_after_a_refund_of_another_line_reports_the_whole_spent_purchase() -> TestResult
{
    let setup = setup().await;
    let support = line("Support", 2_000, false);
    let support_line = support.local_id.clone();
    let invoice = draft(&setup, vec![credit_pack(5_000), support]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    pay_amount(&setup, invoice.id, 7_000).await?;
    assert_eq!(balance(&setup).await, 5_000);
    spend(&setup, 5_000).await?;
    assert_eq!(balance(&setup).await, 0);

    let refund = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: support_line,
                    amount_cents: 2_000,
                }],
                reason: Some("support not delivered".into()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    let payment = settled_payment(&setup, invoice.id).await?;
    setup
        .services
        .record_offline_refund(
            Actor::System,
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: payment,
                amount: 2_000,
                credit_note_id: Some(refund.id),
                refund_date: chrono::Utc::now().naive_utc(),
                reference: Some("wire-out".to_owned()),
                reason: None,
            },
        )
        .await
        .map_err(test_error)?;

    charge_back(&setup, payment, 5_000, chrono::Utc::now().naive_utc()).await?;
    let disputed = setup
        .store
        .get_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (disputed.amount_due, disputed.payment_status),
        (0, InvoicePaymentStatus::Paid),
        "a dispute never reopens the invoice"
    );
    assert_eq!(
        figures(&notices(&setup, invoice.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 0, 5_000)]
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A 5000 purchase with its own grant, 4000 spent, charged back 30% then 70%. Old code reported
/// the cumulative shortfall on each reversal (500 then 4000): 4500 for a 4000 loss.
#[tokio::test]
async fn partial_chargebacks_of_a_granted_purchase_report_each_increment_once() -> TestResult {
    let setup = setup().await;
    let invoice = draft(&setup, vec![credit_pack(5_000)]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    pay_amount(&setup, invoice.id, 5_000).await?;
    spend(&setup, 4_000).await?;
    assert_eq!(balance(&setup).await, 1_000);
    let payment = settled_payment(&setup, invoice.id).await?;

    let first_at = chrono::Utc::now().naive_utc();
    charge_back(&setup, payment, 1_500, first_at).await?;
    charge_back(
        &setup,
        payment,
        3_500,
        first_at + chrono::Duration::seconds(1),
    )
    .await?;
    assert_eq!(balance(&setup).await, 0);
    let reported = notices(&setup, invoice.id).await?;
    assert_eq!(
        figures(&reported),
        vec![
            (CreditUnrecoveredCause::PaymentDisputed, 1_000, 500),
            (CreditUnrecoveredCause::PaymentDisputed, 0, 3_500),
        ]
    );
    assert_eq!(
        reported
            .iter()
            .map(|notice| notice.unrecovered_minor)
            .sum::<i64>(),
        4_000,
        "the notices sum to the spent credit, nothing counted twice"
    );
    assert_eq!(totals(&reported), vec![500, 4_000]);

    // The dispute is won: the unspent credit comes back and the whole loss is withdrawn.
    reinstate(
        &setup,
        payment,
        None,
        first_at + chrono::Duration::seconds(2),
    )
    .await?;
    assert_eq!(balance(&setup).await, 1_000);
    assert_eq!(
        recovered(&recoveries(&setup, invoice.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 4_000, 0)]
    );
    let restored = setup
        .store
        .get_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?;
    assert_eq!(restored.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(notices(&setup, invoice.id).await?.len(), 2);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// 4000 of a 5000 purchase spent, the whole payment charged back, then the dispute won in two
/// parts. Old code left the 4000 notice standing, so the merchant believed 4000 was lost.
#[tokio::test]
async fn a_dispute_won_in_parts_withdraws_the_loss_in_parts() -> TestResult {
    let setup = setup().await;
    let (purchase, _, payment) =
        purchase_spent_on(&setup, 4_000, vec![line("Seats", 4_000, false)]).await?;
    let at = chrono::Utc::now().naive_utc();
    charge_back(&setup, payment, 5_000, at).await?;
    assert_eq!(balance(&setup).await, 0);
    let lost = notices(&setup, purchase.id).await?;
    assert_eq!(
        figures(&lost),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 1_000, 4_000)]
    );
    assert_eq!(totals(&lost), vec![4_000]);

    // 2000 back: 3000 of the 5000 still unbacked, which the 1000 taken back covers to 2000.
    reinstate(
        &setup,
        payment,
        Some(2_000),
        at + chrono::Duration::seconds(1),
    )
    .await?;
    assert_eq!(
        balance(&setup).await,
        0,
        "the taken back credit still covers the rest"
    );
    assert_eq!(
        recovered(&recoveries(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 2_000, 2_000)]
    );

    reinstate(&setup, payment, None, at + chrono::Duration::seconds(2)).await?;
    assert_eq!(balance(&setup).await, 1_000);
    let back = recoveries(&setup, purchase.id).await?;
    assert_eq!(
        recovered(&back),
        vec![
            (CreditUnrecoveredCause::PaymentDisputed, 2_000, 2_000),
            (CreditUnrecoveredCause::PaymentDisputed, 2_000, 0),
        ]
    );
    assert_ne!(
        back[0].id, back[1].id,
        "distinct event ids, so both are delivered"
    );
    assert_eq!(
        notices(&setup, purchase.id).await?.len(),
        1,
        "money back never reports a loss"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The reopened purchase invoice is paid again after its chargeback: the money is back, so is the
/// unspent credit, and the 4000 loss is withdrawn.
#[tokio::test]
async fn paying_a_charged_back_purchase_again_withdraws_its_loss() -> TestResult {
    let setup = setup().await;
    let (purchase, _, payment) =
        purchase_spent_on(&setup, 4_000, vec![line("Seats", 4_000, false)]).await?;
    fail_collection(&setup, payment, 5_000, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(
        figures(&notices(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentReversed, 1_000, 4_000)]
    );

    setup
        .services
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            purchase.id,
            Decimal::new(5_000, 2),
            chrono::Utc::now().fixed_offset(),
            Some(format!("repayment-{}", purchase.id)),
        )
        .await
        .map_err(test_error)?;
    run_once_invoice_orchestration(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
    )
    .await;
    let repaid = setup
        .store
        .get_invoice_by_id(TENANT_ID, purchase.id)
        .await
        .map_err(test_error)?;
    assert_eq!(repaid.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(balance(&setup).await, 1_000);
    assert_eq!(
        recovered(&recoveries(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentReversed, 4_000, 0)]
    );
    assert_eq!(notices(&setup, purchase.id).await?.len(), 1);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The whole 5000 purchase pays part of a 9000 invoice, then the purchase is charged back: all of
/// it is lost. Voiding the 9000 invoice gives the 5000 back to the purchase, which takes it back
/// at once and withdraws the loss; a sweep afterwards finds nothing left to do.
#[tokio::test]
async fn voiding_the_invoice_that_spent_the_credit_withdraws_the_loss() -> TestResult {
    let setup = setup().await;
    let (purchase, spend, payment) =
        purchase_spent_on(&setup, 5_000, vec![line("Seats", 9_000, false)]).await?;
    assert_eq!(spend.amount_due, 4_000);
    charge_back(&setup, payment, 5_000, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(
        figures(&notices(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 0, 5_000)]
    );

    setup
        .store
        .void_invoice(Actor::System, spend.id, TENANT_ID)
        .await
        .map_err(test_error)?;
    assert_eq!(
        balance(&setup).await,
        0,
        "the credit given back is unbacked and taken back"
    );
    assert_eq!(
        recovered(&recoveries(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 5_000, 0)]
    );

    assert_eq!(sweep(&setup).await?, 0);
    assert_eq!(notices(&setup, purchase.id).await?.len(), 1);
    assert_eq!(recoveries(&setup, purchase.id).await?.len(), 1);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The whole 5000 purchase spent across two lines, then charged back (5000 lost). A credit note
/// on the spending invoice gives 2000 of it back to the purchase, which takes it back at once and
/// reports the recovery. Old code left the 2000 spendable until the sweep ran.
#[tokio::test]
async fn a_credit_note_giving_credit_back_to_a_charged_back_purchase_recovers_at_once() -> TestResult
{
    let setup = setup().await;
    let support = line("Support", 2_000, false);
    let support_line = support.local_id.clone();
    let (purchase, spend, payment) =
        purchase_spent_on(&setup, 5_000, vec![line("Seats", 3_000, false), support]).await?;
    charge_back(&setup, payment, 5_000, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(
        figures(&notices(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 0, 5_000)]
    );
    // Credit covered it in full; billing marks such an invoice paid, which a manual draft skips.
    setup
        .store
        .get_conn()
        .await
        .map_err(test_error)?
        .batch_execute(&format!(
            "UPDATE invoice SET payment_status = 'PAID' WHERE tenant_id = '{}' AND id = '{}' AND amount_due = 0",
            TENANT_ID.as_uuid(),
            spend.id.as_uuid()
        ))
        .await
        .map_err(test_error)?;

    let note = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: spend.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: support_line,
                    amount_cents: 2_000,
                }],
                reason: Some("support not delivered".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!(note.credited_amount_cents, 2_000);
    assert_eq!(
        balance(&setup).await,
        0,
        "the credit given back is unbacked and taken back"
    );
    assert_eq!(
        recovered(&recoveries(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentDisputed, 2_000, 3_000)]
    );
    assert_eq!(notices(&setup, purchase.id).await?.len(), 1);

    assert_eq!(sweep(&setup).await?, 0, "nothing is left for the sweep");
    assert_eq!(recoveries(&setup, purchase.id).await?.len(), 1);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[derive(Debug, PartialEq, Eq, diesel::QueryableByName)]
struct GrantReversal {
    #[diesel(sql_type = BigInt)]
    cost: i64,
    #[diesel(sql_type = BigInt)]
    taken: i64,
    #[diesel(sql_type = BigInt)]
    cost_taken: i64,
}

/// Per purchase grant of the invoice, cheapest first: its cost, and the credit and cost payment
/// reversals took back from it.
async fn grant_reversals(
    setup: &MeteroidSetup,
    invoice: InvoiceId,
) -> TestResult<Vec<GrantReversal>> {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::sql_query(
        "SELECT coalesce(g.cost_minor, 0)::bigint AS cost,
                coalesce(-sum(e.amount), 0)::bigint AS taken,
                coalesce(-sum(e.cost_minor), 0)::bigint AS cost_taken
         FROM customer_credit_grant g
         LEFT JOIN customer_credit_entry e
           ON e.grant_id = g.id AND e.kind = 'REVERSAL' AND e.credit_note_id IS NULL
         WHERE g.tenant_id = $1 AND g.invoice_id = $2 AND g.source_kind = 'CREDIT_PURCHASE'
         GROUP BY g.id, g.cost_minor
         ORDER BY g.cost_minor",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.as_uuid())
    .load(&mut conn)
    .await
    .map_err(test_error)
}

/// One invoice sells 1000 credits for 1000 and 1000 for 5000; half of the 6000 is charged back.
/// Old code pooled both grants into one 1000 target filled in grant order, taking 1000 or 5000 of
/// cost back for 3000 of money lost. Each grant loses half instead: 3000 of cost.
#[tokio::test]
async fn a_partial_chargeback_of_two_packs_takes_back_half_of_each() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(
        &["I7"],
        "sells a currency pack above its face through store internals, which no API allows",
    );
    let premium = LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(1_000),
        ..line("Premium pack", 5_000, false)
    };
    let invoice = draft(&setup, vec![credit_pack(1_000), premium]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    pay_amount(&setup, invoice.id, 6_000).await?;
    assert_eq!(balance(&setup).await, 2_000);
    let payment = settled_payment(&setup, invoice.id).await?;

    charge_back(&setup, payment, 3_000, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(balance(&setup).await, 1_000);
    assert_eq!(
        grant_reversals(&setup, invoice.id).await?,
        vec![
            GrantReversal {
                cost: 1_000,
                taken: 500,
                cost_taken: 500,
            },
            GrantReversal {
                cost: 5_000,
                taken: 500,
                cost_taken: 2_500,
            },
        ]
    );
    assert!(
        notices(&setup, invoice.id).await?.is_empty(),
        "nothing spent"
    );

    assert_eq!(
        sweep(&setup).await?,
        0,
        "the sweep agrees with each grant's share"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A paid 7000 invoice: 5000 of credit, all spent, next to a 2000 support line.
async fn spent_purchase_with_support(
    setup: &MeteroidSetup,
) -> TestResult<(Invoice, String, PaymentTransactionId)> {
    let support = line("Support", 2_000, false);
    let support_line = support.local_id.clone();
    let invoice = draft(setup, vec![credit_pack(5_000), support]).await?;
    let invoice = finalize(setup, invoice.id).await?.invoice;
    pay_amount(setup, invoice.id, 7_000).await?;
    spend(setup, 5_000).await?;
    assert_eq!(balance(setup).await, 0);
    let payment = settled_payment(setup, invoice.id).await?;
    Ok((invoice, support_line, payment))
}

async fn credit_support(
    setup: &MeteroidSetup,
    invoice: InvoiceId,
    support_line: String,
    credit_type: CreditType,
) -> TestResult<meteroid_store::domain::CreditNote> {
    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice,
                line_items: vec![CreditLineItem::Amount {
                    local_id: support_line,
                    amount_cents: 2_000,
                }],
                reason: Some("support not delivered".into()),
                memo: None,
                credit_type,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)
}

/// Half of the 7000 charged back (2500 of the spent credit lost), then the support line refunded
/// in cash: the purchase now stands on 5000, 3500 of it missing. The note's cash is the
/// customer's from its finalization, so the loss reaches 3500 then and never dips meanwhile.
#[tokio::test]
async fn a_refund_of_another_line_after_a_chargeback_raises_the_reported_loss() -> TestResult {
    let setup = setup().await;
    let (invoice, support_line, payment) = spent_purchase_with_support(&setup).await?;
    fail_collection(&setup, payment, 3_500, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(totals(&notices(&setup, invoice.id).await?), vec![2_500]);

    let refund = credit_support(&setup, invoice.id, support_line, CreditType::Refund).await?;
    assert!(recoveries(&setup, invoice.id).await?.is_empty());
    assert_eq!(
        totals(&notices(&setup, invoice.id).await?),
        vec![2_500, 3_500]
    );
    setup
        .services
        .record_offline_refund(
            Actor::System,
            TENANT_ID,
            OfflineRefundCommand {
                parent_transaction_id: payment,
                amount: 2_000,
                credit_note_id: Some(refund.id),
                refund_date: chrono::Utc::now().naive_utc(),
                reference: Some("wire-out".to_owned()),
                reason: None,
            },
        )
        .await
        .map_err(test_error)?;
    let reopened = setup
        .store
        .get_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?;
    assert_eq!(reopened.amount_due, 3_500);
    assert_eq!(
        totals(&notices(&setup, invoice.id).await?),
        vec![2_500, 3_500]
    );

    assert_eq!(sweep(&setup).await?, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Half of the 7000 charged back (2500 lost), then the support debt forgiven: the purchase stands
/// on 5000 of which 1500 is still owed. Old code kept reporting 2500.
#[tokio::test]
async fn debt_forgiven_on_another_line_after_a_chargeback_lowers_the_reported_loss() -> TestResult {
    let setup = setup().await;
    let (invoice, support_line, payment) = spent_purchase_with_support(&setup).await?;
    fail_collection(&setup, payment, 3_500, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(totals(&notices(&setup, invoice.id).await?), vec![2_500]);

    let note = credit_support(
        &setup,
        invoice.id,
        support_line,
        CreditType::CreditToBalance,
    )
    .await?;
    assert_eq!(note.credited_amount_cents, 0, "all of it was owed");
    assert_eq!(
        recovered(&recoveries(&setup, invoice.id).await?),
        vec![(CreditUnrecoveredCause::PaymentReversed, 1_000, 1_500)]
    );
    assert_eq!(notices(&setup, invoice.id).await?.len(), 1);

    assert_eq!(sweep(&setup).await?, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A reported loss that no longer matches the ledger (a write lost before this release): the
/// sweep moves it back to the real figure once. Old code never compared the reported total.
#[tokio::test]
async fn the_sweep_repairs_a_stale_unrecovered_total() -> TestResult {
    let setup = setup().await;
    let (invoice, _, payment) = spent_purchase_with_support(&setup).await?;
    charge_back(&setup, payment, 3_500, chrono::Utc::now().naive_utc()).await?;
    assert_eq!(totals(&notices(&setup, invoice.id).await?), vec![2_500]);
    setup
        .store
        .get_conn()
        .await
        .map_err(test_error)?
        .batch_execute(&format!(
            "UPDATE customer_credit_unrecovered SET reported = 1000
             WHERE tenant_id = '{}' AND invoice_id = '{}' AND source_id = invoice_id",
            TENANT_ID.as_uuid(),
            invoice.id.as_uuid()
        ))
        .await
        .map_err(test_error)?;

    assert_eq!(sweep(&setup).await?, 1);
    let lost = notices(&setup, invoice.id).await?;
    assert_eq!(totals(&lost), vec![2_500, 2_500]);
    assert_eq!(lost[1].unrecovered_minor, 1_500);

    assert_eq!(sweep(&setup).await?, 0, "a rerun is a no-op");
    assert_eq!(notices(&setup, invoice.id).await?.len(), 2);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The previous release reverses a payment whose purchased credit is all spent, moving no credit:
/// nothing is left to take back, but the loss is reported once. Old code skipped such an invoice.
#[tokio::test]
async fn the_sweep_reports_a_spent_purchase_the_previous_release_reversed() -> TestResult {
    let setup = setup().await;
    let (purchase, _, _) =
        purchase_spent_on(&setup, 5_000, vec![line("Seats", 5_000, false)]).await?;
    assert_eq!(sweep(&setup).await?, 0, "nothing reversed, nothing to do");
    setup
        .store
        .get_conn()
        .await
        .map_err(test_error)?
        .batch_execute(&format!(
            "UPDATE payment_transaction SET amount_reversed = amount, status = 'REFUNDED', reversal_kind = 'CHARGEBACK'
             WHERE tenant_id = '{tenant}' AND invoice_id = '{invoice}';
             UPDATE invoice SET amount_due = total, payment_status = 'UNPAID'
             WHERE tenant_id = '{tenant}' AND id = '{invoice}';",
            tenant = TENANT_ID.as_uuid(),
            invoice = purchase.id.as_uuid()
        ))
        .await
        .map_err(test_error)?;

    assert_eq!(sweep(&setup).await?, 1);
    assert_eq!(
        figures(&notices(&setup, purchase.id).await?),
        vec![(CreditUnrecoveredCause::PaymentReversed, 0, 5_000)]
    );
    assert_eq!(sweep(&setup).await?, 0, "a rerun is a no-op");
    assert_eq!(notices(&setup, purchase.id).await?.len(), 1);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[derive(diesel::QueryableByName)]
struct Unrecovered {
    #[diesel(sql_type = BigInt)]
    total: i64,
}

async fn unrecovered_total(setup: &MeteroidSetup, invoice: InvoiceId) -> TestResult<i64> {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    Ok(diesel::sql_query(
        "SELECT coalesce(sum(reported), 0)::bigint AS total FROM customer_credit_unrecovered
         WHERE tenant_id = $1 AND invoice_id = $2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.as_uuid())
    .get_result::<Unrecovered>(&mut conn)
    .await
    .map_err(test_error)?
    .total)
}

/// A chargeback of the purchase (purchase invoice, then its grants) races a credit note giving
/// credit back to it (grants, then the purchase invoice). Either order ends at 2000 taken back
/// and 3000 reported, and neither transaction is chosen as a deadlock victim.
#[tokio::test]
async fn a_chargeback_racing_a_recovering_credit_note_never_deadlocks() -> TestResult {
    use scoped_futures::ScopedFutureExt;
    let setup = setup().await;
    for _ in 0..20 {
        let support = line("Support", 2_000, false);
        let support_line = support.local_id.clone();
        let (purchase, spend, payment) =
            purchase_spent_on(&setup, 5_000, vec![line("Seats", 3_000, false), support]).await?;
        setup
            .store
            .get_conn()
            .await
            .map_err(test_error)?
            .batch_execute(&format!(
                "UPDATE invoice SET payment_status = 'PAID' WHERE tenant_id = '{}' AND id = '{}' AND amount_due = 0",
                TENANT_ID.as_uuid(),
                spend.id.as_uuid()
            ))
            .await
            .map_err(test_error)?;

        let store = &setup.store;
        let charge_back = store.transaction(|conn| {
            async move {
                let transaction = store
                    .get_payment_tx_by_id_for_update(conn, payment, TENANT_ID)
                    .await?;
                store
                    .reverse_transaction_tx(
                        conn,
                        &Actor::System,
                        transaction,
                        TransactionReversal {
                            external_transaction_id: "ch_race".to_owned(),
                            amount: ReversalAmount::Full,
                            kind: ReversalKind::Chargeback,
                            reason: Some("dispute lost".to_owned()),
                            reversed_at: chrono::Utc::now().naive_utc(),
                            disputed: true,
                            provider_ref: None,
                        },
                    )
                    .await
            }
            .scope_boxed()
        });
        let note = store.create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: spend.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: support_line,
                    amount_cents: 2_000,
                }],
                reason: Some("support not delivered".into()),
                memo: None,
                credit_type: CreditType::CreditToBalance,
                custom_properties: None,
            },
        );
        let (charged_back, note) = tokio::join!(charge_back, note);
        charged_back.map_err(test_error)?;
        assert_eq!(note.map_err(test_error)?.credited_amount_cents, 2_000);

        assert_eq!(
            balance(&setup).await,
            0,
            "the credit given back is taken back"
        );
        assert_eq!(unrecovered_total(&setup, purchase.id).await?, 3_000);
        assert_eq!(
            grant_reversals(&setup, purchase.id)
                .await?
                .iter()
                .map(|grant| grant.taken)
                .collect::<Vec<_>>(),
            vec![2_000]
        );
        assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    }
    Ok(())
}
