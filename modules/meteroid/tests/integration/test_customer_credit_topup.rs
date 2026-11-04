//! Wallet top-ups on the invoice-time credit ledger: a purchase is charged on a pinned provider
//! payment, funds the wallet exactly once through the invoice-paid path, and every failure mode
//! either retries the same charge or leaves a trace a person can act on.
use crate::data::ids::{
    CUST_SPOTIFY_ID, CUST_SPOTIFY_PAYMENT_METHOD_ID, CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID,
    TENANT_ID,
};
use crate::data::payment::set_mock_charge_behavior;
use crate::helpers::credit::{assert_ledger_balances, available};
use crate::helpers::top_ups::{drain, operations, run_executor, setup, wallet};
use crate::meteroid_it::container::MeteroidSetup;
use crate::test_customer_credit::{draft, finalize, line, pay_amount};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CreditGrantId, CustomerId, InvoiceId, PaymentTransactionId, WalletId,
};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::customer_credit::CustomerCreditOperationRow;
use meteroid_store::adapters::payment::{MockProviderOperation, mock_provider_invocations};
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::credit::{
    CreditGrantSource, GrantClassification, SelfServiceTopUpState, TopUpWallet,
};
use meteroid_store::domain::enums::{
    DeclineKind, InvoicePaymentStatus, InvoiceStatusEnum, ReversalKind,
};
use meteroid_store::domain::outbox_event::{OutboxEvent, PaymentTransactionEvent};
use meteroid_store::domain::payment_transactions::{ObservedRefundStatus, PaymentIntent};
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::wallet_events::CreditUnrecoveredCause;
use meteroid_store::domain::{PaymentStatusEnum, PaymentTypeEnum};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::payment_transactions::{
    ObservedRefund, ObservedRefunds, PaymentTransactionInterface, ReversalAmount,
    TransactionReversal,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::services::ReverseManualPaymentCommand;
use scoped_futures::ScopedFutureExt;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn balance(setup: &MeteroidSetup, customer: CustomerId) -> i64 {
    available(&setup.store, TENANT_ID, customer, None).await
}

fn top_up(customer: CustomerId, wallet: WalletId, amount_minor: i64, key: &str) -> TopUpWallet {
    TopUpWallet {
        tenant_id: TENANT_ID,
        customer_id: customer,
        wallet_id: wallet,
        amount_minor,
        payment_method_id: if customer == CUST_UBER_ID {
            CUST_UBER_PAYMENT_METHOD_ID
        } else {
            CUST_SPOTIFY_PAYMENT_METHOD_ID
        },
        idempotency_key: key.to_owned(),
        expires_at: None,
        unit_price: None,
        expected_unit_price: None,
    }
}

async fn execute(setup: &MeteroidSetup, sql: String) -> TestResult {
    use diesel_async::SimpleAsyncConnection;
    setup
        .store
        .get_conn()
        .await
        .map_err(test_error)?
        .batch_execute(&sql)
        .await
        .map_err(test_error)
}

/// Every event the webhook queue holds, decoded back into the published outbox event.
async fn webhook_events(setup: &MeteroidSetup) -> TestResult<Vec<OutboxEvent>> {
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
        .collect())
}

/// How many `payment.failed` events the webhook queue holds for one payment.
async fn payment_failed_events(
    setup: &MeteroidSetup,
    payment: PaymentTransactionId,
) -> TestResult<usize> {
    Ok(webhook_events(setup)
        .await?
        .iter()
        .filter(|event| {
            matches!(event, OutboxEvent::PaymentTransactionSaved(saved)
                if saved.payment_transaction_id == payment
                    && saved.status == PaymentStatusEnum::Failed)
        })
        .count())
}

fn captures(payment: PaymentTransactionId) -> usize {
    mock_provider_invocations()
        .iter()
        .filter(|call| call.transaction_id == payment && call.captured)
        .count()
}

fn prepares(payment: PaymentTransactionId) -> usize {
    mock_provider_invocations()
        .iter()
        .filter(|call| {
            call.transaction_id == payment && call.operation == MockProviderOperation::Prepare
        })
        .count()
}

/// A provider answer arriving after the fact, as its webhook would record it.
async fn capture_late(setup: &MeteroidSetup, payment: PaymentTransactionId) -> TestResult {
    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, payment)
        .await
        .map_err(test_error)?;
    let intent = PaymentIntent {
        external_id: transaction
            .provider_transaction_id
            .clone()
            .unwrap_or_else(|| format!("late_{}", payment.as_uuid())),
        transaction_id: payment,
        tenant_id: TENANT_ID,
        amount_requested: transaction.amount,
        amount_received: Some(transaction.amount),
        currency: transaction.currency.clone(),
        next_action: None,
        status: PaymentStatusEnum::Settled,
        last_payment_error: None,
        decline_kind: None,
        processed_at: Some(chrono::Utc::now().naive_utc()),
    };
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    setup
        .store
        .consolidate_intent_and_transaction_tx(&mut conn, &Actor::System, transaction, intent)
        .await
        .map_err(test_error)?;
    drop(conn);
    drain(setup).await;
    Ok(())
}

#[tokio::test]
async fn a_top_up_funds_the_wallet_once_and_replays_its_charge() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let quote = setup
        .services
        .quote_wallet_top_up(TENANT_ID, CUST_UBER_ID, wallet, 5_000)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (quote.charge_minor, quote.net_minor, quote.tax_minor),
        (5_000, 5_000, 0)
    );
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "top-up-1"),
            Some(quote.clone()),
        )
        .await
        .map_err(test_error)?;
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Settled);
    assert_eq!(receipt.transaction.payment_type, PaymentTypeEnum::TopUp);
    drain(&setup).await;

    assert_eq!(balance(&setup, CUST_UBER_ID).await, 5_000);
    let invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(
        (invoice.total, invoice.tax_amount, invoice.applied_credits),
        (5_000, 0, 0)
    );
    let wallets = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    let grant = &wallets[0].grants[0];
    assert_eq!(grant.source, CreditGrantSource::Purchase);
    assert_eq!(grant.classification, GrantClassification::Paid);
    assert_eq!(grant.invoice_id, Some(receipt.invoice_id));
    let operation = &operations(&setup, CUST_UBER_ID).await?[0];
    assert_eq!(operation.state, "SETTLED");
    assert_eq!(receipt.transaction.amount, 5_000);

    let replay = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "top-up-1"), Some(quote))
        .await
        .map_err(test_error)?;
    assert_eq!(replay.transaction.id, receipt.transaction.id);
    drain(&setup).await;
    assert_eq!(
        captures(receipt.transaction.id),
        1,
        "one charge, however often it is asked"
    );
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 5_000);

    let changed = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 7_000, "top-up-1"), None)
        .await
        .expect_err("a key names one request");
    assert!(matches!(changed.current_context(), StoreError::Conflict(_)));

    let funded = webhook_events(&setup)
        .await?
        .into_iter()
        .filter_map(|event| match event {
            OutboxEvent::WalletFunded(event) if event.wallet_id == wallet => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(funded.len(), 1);
    assert_eq!(funded[0].amount_minor, 5_000);
    assert_eq!(funded[0].source, CreditGrantSource::Purchase);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Old code dropped the command's `expires_at` on the floor: the charge-now purchase line never
/// carried a term, so the funded grant never expired.
#[tokio::test]
async fn a_charge_now_top_up_funds_a_grant_that_expires_as_requested() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let expires_at = chrono::Utc::now() + chrono::Duration::days(30);

    let past = setup
        .services
        .top_up_wallet(
            TopUpWallet {
                expires_at: Some(chrono::Utc::now() - chrono::Duration::days(1)),
                ..top_up(CUST_UBER_ID, wallet, 5_000, "expiry-past")
            },
            None,
        )
        .await
        .expect_err("an expiry already past is refused before anything is charged");
    assert!(matches!(
        past.current_context(),
        StoreError::InvalidArgument(_)
    ));

    let receipt = setup
        .services
        .top_up_wallet(
            TopUpWallet {
                expires_at: Some(expires_at),
                ..top_up(CUST_UBER_ID, wallet, 5_000, "expiry-future")
            },
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;

    let wallets = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    let grant = &wallets[0].grants[0];
    assert_eq!(grant.invoice_id, Some(receipt.invoice_id));
    assert_eq!(
        grant.expires_at.map(|at| at.timestamp_micros()),
        Some(expires_at.timestamp_micros())
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_quote_refuses_what_the_purchase_would_refuse() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let too_small = setup
        .services
        .quote_wallet_top_up(TENANT_ID, CUST_UBER_ID, wallet, 99)
        .await
        .expect_err("card networks decline dust");
    assert!(matches!(
        too_small.current_context(),
        StoreError::InvalidArgument(_)
    ));
    let foreign = setup
        .services
        .quote_wallet_top_up(TENANT_ID, CUST_SPOTIFY_ID, wallet, 5_000)
        .await
        .expect_err("another customer's wallet");
    assert!(matches!(
        foreign.current_context(),
        StoreError::ValueNotFound(_)
    ));
    Ok(())
}

/// Two customers may choose the same key: each names its own purchase.
#[tokio::test]
async fn an_idempotency_key_belongs_to_its_customer() -> TestResult {
    let setup = setup().await;
    let uber = wallet(&setup, CUST_UBER_ID).await?;
    let spotify = wallet(&setup, CUST_SPOTIFY_ID).await?;
    let first = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, uber, 5_000, "shared-key"), None)
        .await
        .map_err(test_error)?;
    let second = setup
        .services
        .top_up_wallet(top_up(CUST_SPOTIFY_ID, spotify, 3_000, "shared-key"), None)
        .await
        .map_err(test_error)?;
    assert_ne!(first.transaction.id, second.transaction.id);
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 5_000);
    assert_eq!(balance(&setup, CUST_SPOTIFY_ID).await, 3_000);
    Ok(())
}

#[tokio::test]
async fn a_declined_top_up_leaves_no_document_and_its_key_retries() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let declined = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "retry-key"), None)
        .await
        .expect_err("the card is declined");
    assert!(matches!(
        declined.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    let failed = operations(&setup, CUST_UBER_ID).await?.remove(0);
    assert_eq!(failed.state, "FAILED");
    assert!(
        failed.invoice_id.is_none(),
        "the draft document was deleted"
    );
    let first_payment = failed.payment_transaction_id.ok_or("no payment")?;
    let payment = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, first_payment)
        .await
        .map_err(test_error)?;
    assert_eq!(payment.status, PaymentStatusEnum::Failed);
    assert_eq!(payment.decline_kind, Some(DeclineKind::DoNotHonor));
    assert!(payment.invoice_id.is_none());
    assert_eq!(payment_failed_events(&setup, first_payment).await?, 1);
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let state = |key: &str| {
        setup
            .services
            .self_service_top_up_state(TENANT_ID, CUST_UBER_ID, wallet, key.to_owned())
    };
    assert!(matches!(
        state("retry-key").await.map_err(test_error)?,
        Some(SelfServiceTopUpState::Failed { .. })
    ));
    assert_eq!(state("never-sent").await.map_err(test_error)?, None);

    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Succeeded).await;
    let retried = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "retry-key"), None)
        .await
        .map_err(test_error)?;
    assert_ne!(retried.transaction.id, first_payment);
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 5_000);
    let operations = operations(&setup, CUST_UBER_ID).await?;
    assert_eq!(operations.len(), 1, "one purchase per key");
    assert_eq!(operations[0].state, "SETTLED");
    assert_eq!(
        setup
            .services
            .self_service_top_up_state(TENANT_ID, CUST_UBER_ID, wallet, "retry-key".to_owned())
            .await
            .map_err(test_error)?,
        Some(SelfServiceTopUpState::Succeeded)
    );
    assert_eq!(
        operations[0].metadata[CustomerCreditOperationRow::SUPERSEDED_PAYMENTS_KEY],
        serde_json::json!([first_payment.to_string()])
    );
    Ok(())
}

/// Retrying a declined key charges again, so each retry counts toward the daily limit: a card
/// cannot be tested by retrying one top-up.
#[tokio::test]
async fn declined_retries_count_toward_the_daily_top_up_limit() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let rate_limited = |error: &error_stack::Report<StoreError>| {
        error.frames().any(|frame| {
            frame
                .downcast_ref::<meteroid_store::errors::RateLimited>()
                .is_some()
        })
    };
    for attempt in 1..=10 {
        let declined = setup
            .services
            .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "card-test"), None)
            .await
            .expect_err("the card is declined");
        assert!(!rate_limited(&declined), "attempt {attempt} is charged");
    }
    let limited = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "card-test"), None)
        .await
        .expect_err("the eleventh attempt is refused");
    assert!(rate_limited(&limited), "{limited:?}");
    let operations = operations(&setup, CUST_UBER_ID).await?;
    assert_eq!(operations.len(), 1, "one purchase per key");
    assert_eq!(
        operations[0].metadata[CustomerCreditOperationRow::SUPERSEDED_PAYMENTS_KEY]
            .as_array()
            .map(Vec::len),
        Some(9),
        "ten charges, the last one current"
    );
    Ok(())
}

/// The provider captured a charge after its attempt was given up: the document is reissued and
/// the money becomes the credit it bought, once.
#[tokio::test]
async fn a_late_capture_reissues_the_purchase_document() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let declined = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 4_000, "late"), None)
        .await
        .expect_err("declined");
    assert!(matches!(
        declined.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    let payment = operations(&setup, CUST_UBER_ID).await?[0]
        .payment_transaction_id
        .ok_or("no payment")?;

    capture_late(&setup, payment).await?;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 4_000);
    let operation = operations(&setup, CUST_UBER_ID).await?.remove(0);
    assert_eq!(operation.state, "SETTLED");
    let invoice_id = operation.invoice_id.ok_or("no reissued document")?;
    let invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, invoice_id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 4_000, "funded once");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A settlement event queued while the purchase still had its document names a deleted invoice:
/// the payment row, not the event, says where the money belongs.
#[tokio::test]
async fn a_settlement_event_naming_a_discarded_document_still_funds_the_top_up() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let _declined = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 4_000, "stale-event"), None)
        .await
        .expect_err("declined");
    let operation = operations(&setup, CUST_UBER_ID).await?.remove(0);
    let payment = operation.payment_transaction_id.ok_or("no payment")?;
    let discarded = InvoiceId::new();

    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, payment)
        .await
        .map_err(test_error)?;
    let intent = PaymentIntent {
        external_id: format!("late_{}", payment.as_uuid()),
        transaction_id: payment,
        tenant_id: TENANT_ID,
        amount_requested: transaction.amount,
        amount_received: Some(transaction.amount),
        currency: transaction.currency.clone(),
        next_action: None,
        status: PaymentStatusEnum::Settled,
        last_payment_error: None,
        decline_kind: None,
        processed_at: Some(chrono::Utc::now().naive_utc()),
    };
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let settled = setup
        .store
        .consolidate_intent_and_transaction_tx(&mut conn, &Actor::System, transaction, intent)
        .await
        .map_err(test_error)?;
    drop(conn);
    let mut stale: PaymentTransactionEvent = settled.into();
    stale.invoice_id = Some(discarded);
    setup
        .services
        .on_payment_transaction_settled(stale)
        .await
        .map_err(test_error)?;
    drain(&setup).await;

    assert_eq!(balance(&setup, CUST_UBER_ID).await, 4_000, "funded once");
    assert_eq!(operations(&setup, CUST_UBER_ID).await?[0].state, "SETTLED");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A capture of a charge its key has since replaced is never funded a second time: the customer
/// is refunded the duplicate charge through the connector.
#[tokio::test]
async fn a_superseded_capture_is_refunded() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let declined = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 4_000, "superseded"), None)
        .await
        .expect_err("declined");
    assert!(matches!(
        declined.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    let first = operations(&setup, CUST_UBER_ID).await?[0]
        .payment_transaction_id
        .ok_or("no payment")?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Succeeded).await;
    setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 4_000, "superseded"), None)
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 4_000);

    capture_late(&setup, first).await?;
    assert_eq!(
        balance(&setup, CUST_UBER_ID).await,
        4_000,
        "never funded twice"
    );
    let refunds = refunds_of(&setup, first).await?;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 4_000);
    assert_eq!(
        refunds[0].status,
        diesel_models::enums::PaymentStatusEnum::Settled
    );
    assert!(refunds[0].provider_transaction_id.is_some());
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;

    capture_late(&setup, first).await?;
    assert_eq!(
        refunds_of(&setup, first).await?.len(),
        1,
        "a replayed capture does not refund twice"
    );
    Ok(())
}

/// A charge the provider never answered is resumed on the payment it was pinned to: the intent is
/// prepared once and captured once.
#[tokio::test]
async fn an_unanswered_charge_is_resumed_under_the_same_payment() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Unknown).await;
    let receipt = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 2_500, "unanswered"), None)
        .await
        .map_err(test_error)?;
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Pending);
    assert_eq!(
        operations(&setup, CUST_UBER_ID).await?[0].state,
        "PROCESSING"
    );
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);

    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Succeeded).await;
    execute(
        &setup,
        format!(
            "UPDATE customer_credit_operation SET updated_at = now() - interval '11 minutes' WHERE customer_id = '{}'",
            CUST_UBER_ID.as_uuid()
        ),
    )
    .await?;
    run_executor(&setup).await?;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 2_500);
    assert_eq!(operations(&setup, CUST_UBER_ID).await?[0].state, "SETTLED");
    assert_eq!(prepares(receipt.transaction.id), 1);
    assert_eq!(captures(receipt.transaction.id), 1);
    Ok(())
}

/// Resuming an unanswered charge reuses its payment, so it is one attempt however often it is
/// resumed: the customer can still top up.
#[tokio::test]
async fn resuming_an_unanswered_charge_does_not_use_up_the_daily_limit() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Unknown).await;
    setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 2_500, "unanswered"), None)
        .await
        .map_err(test_error)?;
    for _ in 0..9 {
        execute(
            &setup,
            format!(
                "UPDATE customer_credit_operation SET updated_at = now() - interval '11 minutes' WHERE customer_id = '{}'",
                CUST_UBER_ID.as_uuid()
            ),
        )
        .await?;
        run_executor(&setup).await?;
    }
    let pending = operations(&setup, CUST_UBER_ID).await?.remove(0);
    assert_eq!(pending.state, "PROCESSING");
    assert_eq!(
        pending.metadata[CustomerCreditOperationRow::ATTEMPTS_KEY],
        serde_json::json!(10),
        "resumed nine times"
    );

    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Succeeded).await;
    setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 1_000, "another"), None)
        .await
        .map_err(test_error)?;
    Ok(())
}

/// An authentication the customer never completed ends the purchase: its issued document is
/// voided and nothing is funded.
#[tokio::test]
async fn an_abandoned_top_up_checkout_discards_an_unnumbered_draft() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::RequiresAction).await;
    let receipt = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 3_000, "3ds"), None)
        .await
        .map_err(test_error)?;
    assert!(
        receipt.next_action.is_some(),
        "the customer is handed the challenge"
    );
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Pending);
    let issued = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(issued.status, InvoiceStatusEnum::Draft);
    assert_eq!(issued.invoice_number, "draft");

    execute(
        &setup,
        format!(
            "UPDATE payment_transaction SET created_at = now() - interval '25 hours' WHERE id = '{}';
             UPDATE customer_credit_operation SET updated_at = now() - interval '11 minutes' WHERE customer_id = '{}'",
            receipt.transaction.id.as_uuid(),
            CUST_UBER_ID.as_uuid()
        ),
    )
    .await?;
    run_executor(&setup).await?;
    assert!(
        setup
            .store
            .get_detailed_invoice_by_id(TENANT_ID, receipt.invoice_id)
            .await
            .is_err(),
        "abandonment deletes the unnumbered draft"
    );
    let payment = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, receipt.transaction.id)
        .await
        .map_err(test_error)?;
    assert_eq!(payment.status, PaymentStatusEnum::Failed);
    assert!(payment.invoice_id.is_none());
    assert_eq!(operations(&setup, CUST_UBER_ID).await?[0].state, "FAILED");
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    Ok(())
}

/// The bank claws the whole charge back, as the provider's webhook would record it.
async fn chargeback(setup: &MeteroidSetup, payment: PaymentTransactionId) -> TestResult {
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
                external_transaction_id: "ch_disputed".to_owned(),
                amount: ReversalAmount::Full,
                kind: ReversalKind::Chargeback,
                reason: Some("fraudulent".to_owned()),
                reversed_at: chrono::Utc::now().naive_utc(),
                disputed: true,
                provider_ref: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(())
}

fn unrecovered_notices(
    events: &[OutboxEvent],
    wallet: WalletId,
) -> Vec<(CreditUnrecoveredCause, i64, i64)> {
    events
        .iter()
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditUnrecovered(event) if event.wallet_id == wallet => {
                Some((event.cause, event.taken_back_minor, event.unrecovered_minor))
            }
            _ => None,
        })
        .collect()
}

async fn refunds_of(
    setup: &MeteroidSetup,
    payment: PaymentTransactionId,
) -> TestResult<Vec<diesel_models::payments::PaymentTransactionRow>> {
    use diesel_models::schema::payment_transaction::dsl as p;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    p::payment_transaction
        .filter(p::tenant_id.eq(TENANT_ID))
        .filter(p::parent_transaction_id.eq(payment))
        .filter(p::payment_type.eq(diesel_models::enums::PaymentTypeEnum::Refund))
        .order(p::created_at.asc())
        .select(diesel_models::payments::PaymentTransactionRow::as_select())
        .load(&mut conn)
        .await
        .map_err(test_error)
}

fn debited_by_reversal(events: &[OutboxEvent], wallet: WalletId) -> Vec<i64> {
    events
        .iter()
        .filter_map(|event| match event {
            // A chargeback's debit says the payment was lost to a dispute, a bank return's that
            // the bank reversed it.
            OutboxEvent::WalletDebited(event)
                if event.wallet_id == wallet
                    && matches!(
                        event.reason.as_str(),
                        "Payment reversed by the bank" | "Payment lost to a dispute"
                    ) =>
            {
                Some(event.amount_minor)
            }
            _ => None,
        })
        .collect()
}

/// Part of the purchase was spent and its payment returned by the bank: only the unspent part
/// comes back, the spent part is reported in a wallet event, and paying the reopened invoice
/// restores the credit.
#[tokio::test]
async fn a_reversed_top_up_reports_the_spent_part_and_payment_restores_the_rest() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "spent"), None)
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    let spend = draft(&setup, vec![line("Seats", 2_000, false)]).await?;
    finalize(&setup, spend.id).await?;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);

    // The operator records a bank return the provider never reported.
    setup
        .services
        .reverse_manual_payment(
            Actor::System,
            TENANT_ID,
            ReverseManualPaymentCommand {
                transaction_id: receipt.transaction.id,
                kind: ReversalKind::Returned,
                reason: Some("returned by the bank".to_owned()),
                reversed_at: chrono::Utc::now().naive_utc(),
            },
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    assert_eq!(
        debited_by_reversal(&webhook_events(&setup).await?, wallet),
        vec![3_000]
    );
    assert_eq!(
        unrecovered_notices(&webhook_events(&setup).await?, wallet),
        vec![(CreditUnrecoveredCause::PaymentReversed, 3_000, 2_000)]
    );
    let reopened = setup
        .store
        .get_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .map_err(test_error)?;
    assert_eq!(
        reopened.amount_due, 5_000,
        "the reopened invoice collects it"
    );

    pay_amount(&setup, receipt.invoice_id, 5_000).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Old code: the failure handler locked the top-up then its payment, the chargeback the payment
/// then the top-up, so a stale failure event racing a chargeback deadlocked one of them.
#[tokio::test]
async fn a_chargeback_racing_its_top_ups_failure_handler_never_deadlocks() -> TestResult {
    use diesel_async::SimpleAsyncConnection;
    use std::time::Duration;
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    for iteration in 0..20_u64 {
        let receipt = setup
            .services
            .top_up_wallet(
                top_up(CUST_UBER_ID, wallet, 5_000, &format!("race-{iteration}")),
                None,
            )
            .await
            .map_err(test_error)?;
        drain(&setup).await;
        assert_eq!(balance(&setup, CUST_UBER_ID).await, 5_000);
        let payment = receipt.transaction.id;
        let mut stale: PaymentTransactionEvent = setup
            .store
            .get_payment_tx_by_id(TENANT_ID, payment)
            .await
            .map_err(test_error)?
            .into();
        stale.status = PaymentStatusEnum::Failed;
        stale.error_type = Some("a stale decline".to_owned());

        // Holding the customer queues both behind the chargeback's payment lock.
        let mut gate = setup.store.get_conn().await.map_err(test_error)?;
        gate.batch_execute(&format!(
            "BEGIN; SELECT 1 FROM customer WHERE id = '{}' FOR UPDATE;",
            CUST_UBER_ID.as_uuid()
        ))
        .await
        .map_err(test_error)?;
        let chargeback = setup.services.reverse_manual_payment(
            Actor::System,
            TENANT_ID,
            ReverseManualPaymentCommand {
                transaction_id: payment,
                kind: ReversalKind::Chargeback,
                reason: Some("fraudulent".to_owned()),
                reversed_at: chrono::Utc::now().naive_utc(),
            },
        );
        let handler = async {
            tokio::time::sleep(Duration::from_millis(iteration * 10)).await;
            setup.services.on_payment_transaction_failed(stale).await
        };
        let release = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            gate.batch_execute("COMMIT").await
        };
        let (reversed, handled, released) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(chargeback, handler, release)
        })
        .await?;
        released.map_err(test_error)?;
        reversed.map_err(test_error)?;
        handled.map_err(test_error)?;
        drain(&setup).await;

        let transaction = setup
            .store
            .get_payment_tx_by_id(TENANT_ID, payment)
            .await
            .map_err(test_error)?;
        assert_eq!(transaction.amount_reversed, 5_000);
        let operation = operations(&setup, CUST_UBER_ID)
            .await?
            .into_iter()
            .find(|operation| operation.payment_transaction_id == Some(payment))
            .ok_or("no top-up for the payment")?;
        assert_eq!(operation.state, "SETTLED", "a stale failure ends nothing");
        let invoice = setup
            .store
            .get_invoice_by_id(TENANT_ID, receipt.invoice_id)
            .await
            .map_err(test_error)?;
        assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
        assert_eq!(invoice.amount_due, 0);
        assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
        assert_eq!(payment_failed_events(&setup, payment).await?, 0);
        assert_eq!(
            debited_by_reversal(&webhook_events(&setup).await?, wallet),
            vec![5_000; usize::try_from(iteration + 1)?]
        );
        // Out of the rolling-day purchase and charge limits, so the next iteration may buy again.
        for table in ["customer_credit_operation", "payment_transaction"] {
            execute(
                &setup,
                format!("UPDATE {table} SET created_at = created_at - interval '2 days'"),
            )
            .await?;
        }
    }
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn top_up_checkout_requires_action_settles_once() -> TestResult {
    use meteroid_store::domain::checkout_sessions::{
        CheckoutCompletionResult, CheckoutSessionStatus, CheckoutType,
    };
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::RequiresAction).await;
    let receipt = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 3_000, "checkout-3ds"), None)
        .await
        .map_err(test_error)?;
    let id = receipt
        .transaction
        .checkout_session_id
        .ok_or("missing session")?;
    let session = setup
        .store
        .get_checkout_session(TENANT_ID, id)
        .await
        .map_err(test_error)?;
    assert_eq!(session.checkout_type, CheckoutType::TopUp);
    assert_eq!(session.status, CheckoutSessionStatus::AwaitingPayment);
    assert_eq!(session.plan_version_id, None);
    let invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(invoice.status, InvoiceStatusEnum::Draft);
    assert_eq!(invoice.invoice_number, "draft");
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let retry = setup
        .services
        .complete_checkout(
            TENANT_ID,
            id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3_000,
            receipt.currency,
            None,
        )
        .await
        .map_err(test_error)?;
    match retry {
        CheckoutCompletionResult::AwaitingPayment {
            transaction,
            next_action,
        } => {
            assert_eq!(transaction.id, receipt.transaction.id);
            assert!(next_action.is_some());
        }
        other => panic!("expected awaiting authentication, got {other:?}"),
    }
    capture_late(&setup, receipt.transaction.id).await?;
    capture_late(&setup, receipt.transaction.id).await?;
    let paid = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(paid.status, InvoiceStatusEnum::Finalized);
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    assert_ne!(paid.invoice_number, "draft");
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);
    assert_eq!(
        setup
            .store
            .get_checkout_session(TENANT_ID, id)
            .await
            .map_err(test_error)?
            .status,
        CheckoutSessionStatus::Completed
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn top_up_new_card_checkout_creates_no_purchase_before_confirmation() -> TestResult {
    use meteroid_store::domain::checkout_sessions::{
        CheckoutCompletionResult, CheckoutSessionStatus,
    };
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let session = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            2_000,
            "new-card".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?;
    assert_eq!(session.status, CheckoutSessionStatus::Created);
    let token = meteroid_store::jwt_claims::generate_portal_token(
        &setup.config.jwt_secret,
        TENANT_ID,
        meteroid_store::jwt_claims::ResourceAccess::CheckoutSession(session.id),
        None,
        None,
        false,
    )
    .map_err(test_error)?;
    let mut client = meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_client::PortalCheckoutServiceClient::new(setup.channel.clone());
    let mut request = tonic::Request::new(
        meteroid_grpc::meteroid::portal::checkout::v1::GetCheckoutRequest { coupon_code: None },
    );
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse()?);
    let page = client.get_checkout(request).await?.into_inner();
    assert_eq!(
        page.checkout_type,
        meteroid_grpc::meteroid::portal::checkout::v1::CheckoutType::TopUp as i32
    );
    let checkout = page.checkout.ok_or("missing checkout")?;
    assert!(checkout.subscription.is_none());
    assert_eq!(checkout.amount_due, 2_000);
    assert_eq!(checkout.top_up_currency.as_deref(), Some("EUR"));

    assert!(operations(&setup, CUST_UBER_ID).await?.is_empty());
    let repeated = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            2_000,
            "new-card".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?;
    assert_eq!(session.id, repeated.id);
    let currency = session.top_up().map_err(test_error)?.currency;
    assert!(
        setup
            .services
            .complete_checkout(
                TENANT_ID,
                session.id,
                CUST_UBER_PAYMENT_METHOD_ID,
                2_001,
                currency.clone(),
                None
            )
            .await
            .is_err()
    );
    let completed = setup
        .services
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            2_000,
            currency,
            None,
        )
        .await
        .map_err(test_error)?;
    assert!(matches!(
        completed,
        CheckoutCompletionResult::TopUpCompleted { .. }
    ));
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 2_000);
    Ok(())
}

#[test]
fn credit_webhook_wallet_identity_is_currency_and_scope() {
    use common_domain::ids::{LedgerEntryId, ProductId, SubscriptionId};
    use meteroid::api_rest::webhooks::out_model::{
        WebhookOutCreditGrantExpiredEventData, WebhookOutWalletBalanceLowEventData,
        WebhookOutWalletDebitedEventData, WebhookOutWalletFundedEventData,
    };
    use meteroid_store::domain::credit::GrantScope;
    use meteroid_store::domain::wallet_events::{
        CreditGrantExpiredEvent, GrantLapseReason, WalletBalanceLowEvent, WalletDebitedEvent,
        WalletFundedEvent,
    };
    let subscription = SubscriptionId::new();
    let product = ProductId::new();
    for (scope, expected, subscription_json, products_json) in [
        (
            GrantScope::customer_wide(),
            "ALL",
            serde_json::Value::Null,
            serde_json::json!([]),
        ),
        (
            GrantScope::subscription_usage(subscription).with_products(vec![product]),
            "USAGE",
            serde_json::json!(subscription.as_base62()),
            serde_json::json!([product.as_base62()]),
        ),
    ] {
        let wallet = WalletId::new();
        let entry = LedgerEntryId::new();
        let grant = CreditGrantId::new();
        let payloads = [
            serde_json::to_value(WebhookOutWalletFundedEventData::from(
                WalletFundedEvent::new(
                    TENANT_ID,
                    CUST_UBER_ID,
                    wallet,
                    &scope,
                    entry,
                    grant,
                    None,
                    None,
                    100,
                    "EUR".into(),
                    CreditGrantSource::Manual,
                    GrantClassification::Paid,
                ),
            ))
            .unwrap(),
            serde_json::to_value(WebhookOutWalletDebitedEventData::from(
                WalletDebitedEvent::new(
                    TENANT_ID,
                    CUST_UBER_ID,
                    wallet,
                    &scope,
                    entry,
                    grant,
                    None,
                    None,
                    100,
                    "EUR".into(),
                    "Removed".into(),
                    None,
                ),
            ))
            .unwrap(),
            serde_json::to_value(WebhookOutCreditGrantExpiredEventData::from(
                CreditGrantExpiredEvent::new(
                    TENANT_ID,
                    CUST_UBER_ID,
                    wallet,
                    &scope,
                    grant,
                    entry,
                    100,
                    "EUR".into(),
                    chrono::Utc::now(),
                    GrantLapseReason::SubscriptionEnded,
                ),
            ))
            .unwrap(),
        ];
        for payload in &payloads {
            assert_eq!(payload["currency"], "EUR");
            assert_eq!(payload["applies_to"], expected);
            assert_eq!(payload["subscription_id"], subscription_json);
            assert_eq!(payload["applicable_product_ids"], products_json);
            assert!(payload.get("wallet_id").is_none());
        }
        assert_eq!(payloads[2]["reason"], "SUBSCRIPTION_ENDED");
    }
    // Wallet-level events carry no grant scope: what credit pays is set per grant.
    let low = serde_json::to_value(WebhookOutWalletBalanceLowEventData::from(
        WalletBalanceLowEvent::new(
            TENANT_ID,
            CUST_UBER_ID,
            WalletId::new(),
            "EUR".into(),
            100,
            200,
        ),
    ))
    .unwrap();
    assert!(low.get("applies_to").is_none());
    assert!(low.get("wallet_id").is_none());
}

#[tokio::test]
async fn declined_top_up_checkout_can_retry_with_a_different_card() -> TestResult {
    use common_domain::ids::CustomerPaymentMethodId;
    use diesel_models::customer_payment_methods::{
        CustomerPaymentMethodRow, CustomerPaymentMethodRowNew,
    };
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let session = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            2_000,
            "change-card".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    assert!(
        setup
            .services
            .complete_checkout(
                TENANT_ID,
                session.id,
                CUST_UBER_PAYMENT_METHOD_ID,
                2_000,
                "EUR".into(),
                None
            )
            .await
            .is_err()
    );
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let original =
        CustomerPaymentMethodRow::get_by_id(&mut conn, &TENANT_ID, &CUST_UBER_PAYMENT_METHOD_ID)
            .await
            .map_err(test_error)?;
    let replacement = CustomerPaymentMethodId::new();
    CustomerPaymentMethodRowNew {
        id: replacement,
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        connection_id: original.connection_id,
        external_payment_method_id: "replacement-card".into(),
        payment_method_type: original.payment_method_type,
        account_number_hint: None,
        card_brand: Some("visa".into()),
        card_last4: Some("4242".into()),
        card_exp_month: Some(12),
        card_exp_year: Some(2035),
        fingerprint: None,
    }
    .upsert(&mut conn)
    .await
    .map_err(test_error)?;
    drop(conn);
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Succeeded).await;
    setup
        .services
        .complete_checkout(
            TENANT_ID,
            session.id,
            replacement,
            2_000,
            "EUR".into(),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    let receipt = setup
        .services
        .top_up_checkout_receipt(&session)
        .await
        .map_err(test_error)?
        .ok_or("missing settled receipt")?;
    assert_eq!(receipt.transaction.status, PaymentStatusEnum::Settled);
    assert_eq!(receipt.transaction.payment_method_id, Some(replacement));
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 2_000);
    Ok(())
}

/// The provider reports the payment's refunds reaching `cumulative_minor`, as a refund made in its
/// dashboard does: what no refund row accounts for is recorded as a refund without a credit note.
async fn provider_refund(
    setup: &MeteroidSetup,
    payment: PaymentTransactionId,
    cumulative_minor: i64,
    refund: (&str, ObservedRefundStatus),
) -> TestResult {
    let store = setup.store.clone();
    let (refund_id, status) = (refund.0.to_owned(), refund.1);
    setup
        .store
        .transaction(|conn| {
            async move {
                let parent = store
                    .get_payment_tx_by_id_for_update(conn, payment, TENANT_ID)
                    .await?;
                let observed = ObservedRefunds {
                    external_transaction_id: parent
                        .provider_transaction_id
                        .clone()
                        .unwrap_or_default(),
                    cumulative_refunded_minor: cumulative_minor,
                    refund_ids: vec![ObservedRefund {
                        external_refund_id: refund_id,
                        amount_minor: None,
                        status,
                    }],
                    observed_at: chrono::Utc::now().naive_utc(),
                };
                store
                    .record_observed_refunds_tx(conn, &Actor::System, parent, observed)
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .map_err(test_error)
}

fn debited_by_refund(events: &[OutboxEvent], wallet: WalletId) -> Vec<i64> {
    events
        .iter()
        .filter_map(|event| match event {
            OutboxEvent::WalletDebited(event)
                if event.wallet_id == wallet && event.reason == "Payment refunded" =>
            {
                Some(event.amount_minor)
            }
            _ => None,
        })
        .collect()
}

/// 20.00 of a 50.00 top-up spent, then all of it refunded at the provider: the 30.00 left comes
/// back and the 20.00 spent is reported unrecovered, as refunded.
#[tokio::test]
async fn a_provider_refund_after_spending_reports_the_spent_credit_as_refunded() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "spent then refunded"),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    let spend = draft(&setup, vec![line("Seats", 2_000, false)]).await?;
    finalize(&setup, spend.id).await?;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);

    let refund = ("re_spent", ObservedRefundStatus::Succeeded);
    provider_refund(&setup, receipt.transaction.id, 5_000, refund).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let events = webhook_events(&setup).await?;
    assert_eq!(debited_by_refund(&events, wallet), vec![3_000]);
    assert_eq!(
        unrecovered_notices(&events, wallet),
        vec![(CreditUnrecoveredCause::PaymentRefunded, 3_000, 2_000)]
    );

    provider_refund(&setup, receipt.transaction.id, 5_000, refund).await?;
    assert_eq!(
        unrecovered_notices(&webhook_events(&setup).await?, wallet).len(),
        1,
        "a redelivery reports nothing more"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A refund the provider reports in flight takes the credit back at once; when it fails, the
/// money stayed with the merchant and the credit comes back.
#[tokio::test]
async fn a_failed_provider_refund_gives_the_credit_back() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(top_up(CUST_UBER_ID, wallet, 5_000, "refund failed"), None)
        .await
        .map_err(test_error)?;
    drain(&setup).await;

    let pending = ("re_failing", ObservedRefundStatus::Pending);
    provider_refund(&setup, receipt.transaction.id, 5_000, pending).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);

    let failed = ("re_failing", ObservedRefundStatus::Failed);
    provider_refund(&setup, receipt.transaction.id, 0, failed).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 5_000);
    let refunds = refunds_of(&setup, receipt.transaction.id).await?;
    assert_eq!(
        refunds
            .iter()
            .map(|refund| refund.status.clone())
            .collect::<Vec<_>>(),
        vec![diesel_models::enums::PaymentStatusEnum::Failed]
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// (cause, recovered, loss still reported) of each `wallet.credit_recovered` of the wallet.
fn recovered_notices(
    events: &[OutboxEvent],
    wallet: WalletId,
) -> Vec<(CreditUnrecoveredCause, i64, i64)> {
    events
        .iter()
        .filter_map(|event| match event {
            OutboxEvent::WalletCreditRecovered(event) if event.wallet_id == wallet => Some((
                event.cause,
                event.recovered_minor,
                event.unrecovered_total_minor,
            )),
            _ => None,
        })
        .collect()
}

/// A Refund credit note of `amount_cents` on the top-up's purchase line.
async fn refund_note_on(
    setup: &MeteroidSetup,
    invoice_id: InvoiceId,
    amount_cents: i64,
) -> TestResult<meteroid_store::domain::CreditNote> {
    use meteroid_store::domain::LineItemKind;
    use meteroid_store::domain::enums::CreditType;
    use meteroid_store::repositories::CreditNoteInterface;
    use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditLineItem};
    let invoice = setup
        .store
        .get_invoice_by_id(TENANT_ID, invoice_id)
        .await
        .map_err(test_error)?;
    let purchase_line = invoice
        .line_items
        .iter()
        .find(|line| line.kind == LineItemKind::Credit)
        .ok_or("no purchase line")?;
    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: purchase_line.local_id.clone(),
                    amount_cents,
                }],
                reason: Some("Customer asked for a refund".to_owned()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)
}

/// The ledger reasons of the wallet's credit movements, in order.
async fn ledger_reasons(setup: &MeteroidSetup, wallet: WalletId) -> TestResult<Vec<String>> {
    let (entries, _) = setup
        .store
        .customer_credit_ledger(TENANT_ID, wallet, 1_000, 0)
        .await
        .map_err(test_error)?;
    let mut entries = entries;
    entries.sort_by_key(|entry| entry.sequence);
    Ok(entries
        .into_iter()
        .map(|entry| format!("{:?} {} {}", entry.kind, entry.amount_minor, entry.reason))
        .collect())
}

/// 20.00 of a 50.00 top-up refunded at the provider, then a refund note on the 30.00 left: the
/// note's cash is the customer's from its finalization, so the 20.00 taken back never comes back,
/// whether the note's own refund is paid or not.
#[tokio::test]
async fn a_provider_refund_then_a_refund_note_never_gives_the_credit_back() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "refund then note"),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;

    let refund = ("re_before_note", ObservedRefundStatus::Succeeded);
    provider_refund(&setup, receipt.transaction.id, 2_000, refund).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);

    let note = refund_note_on(&setup, receipt.invoice_id, 3_000).await?;
    drain(&setup).await;
    assert_eq!(note.refunded_amount_cents, 3_000);
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let events = webhook_events(&setup).await?;
    assert_eq!(debited_by_refund(&events, wallet), vec![2_000]);
    assert!(
        !ledger_reasons(&setup, wallet)
            .await?
            .iter()
            .any(|entry| entry.starts_with("Reinstatement")),
        "nothing taken back is restored"
    );
    assert!(unrecovered_notices(&events, wallet).is_empty());
    assert!(recovered_notices(&events, wallet).is_empty());
    assert_eq!(
        setup
            .store
            .reconcile_reversed_credit_purchases(100)
            .await
            .map_err(test_error)?,
        0
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// 20.00 of a 50.00 top-up refunded at the provider, then the whole charge disputed: the dispute
/// can only take the 30.00 the payment still holds, and the credit goes once, 20.00 then 30.00.
#[tokio::test]
async fn a_refund_then_a_chargeback_take_the_top_up_back_once() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "refund then dispute"),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;

    let refund = ("re_before_dispute", ObservedRefundStatus::Succeeded);
    provider_refund(&setup, receipt.transaction.id, 2_000, refund).await?;
    chargeback(&setup, receipt.transaction.id).await?;
    drain(&setup).await;
    let payment = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, receipt.transaction.id)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (
            payment.amount_refunded,
            payment.amount_reversed,
            payment.amount_reversed_excess
        ),
        (2_000, 3_000, 2_000)
    );
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let events = webhook_events(&setup).await?;
    assert_eq!(debited_by_refund(&events, wallet), vec![2_000]);
    assert_eq!(debited_by_reversal(&events, wallet), vec![3_000]);
    assert!(unrecovered_notices(&events, wallet).is_empty());
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// 20.00 of a 50.00 top-up disputed, then the provider reports the whole charge refunded: only the
/// 30.00 the payment still holds is recorded, and the credit goes once, 20.00 then 30.00.
#[tokio::test]
async fn a_chargeback_then_a_refund_take_the_top_up_back_once() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "dispute then refund"),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, receipt.transaction.id)
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
                external_transaction_id: "ch_disputed_then_refunded".to_owned(),
                amount: ReversalAmount::Incremental(2_000),
                kind: ReversalKind::Chargeback,
                reason: Some("fraudulent".to_owned()),
                reversed_at: chrono::Utc::now().naive_utc(),
                disputed: true,
                provider_ref: None,
            },
        )
        .await
        .map_err(test_error)?;
    drop(conn);
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);

    let refund = ("re_after_dispute", ObservedRefundStatus::Succeeded);
    provider_refund(&setup, receipt.transaction.id, 5_000, refund).await?;
    drain(&setup).await;
    assert_eq!(
        refunds_of(&setup, receipt.transaction.id)
            .await?
            .iter()
            .map(|row| row.amount)
            .collect::<Vec<_>>(),
        vec![3_000],
        "bounded by what the payment still held"
    );
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let events = webhook_events(&setup).await?;
    assert_eq!(debited_by_reversal(&events, wallet), vec![2_000]);
    assert_eq!(debited_by_refund(&events, wallet), vec![3_000]);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// 20.00 of a 50.00 top-up spent, then a refund of all of it in flight: the 30.00 left goes and the
/// 20.00 spent is reported. The provider then fails the refund (`refund.updated`): the 30.00 comes
/// back and the report is withdrawn.
#[tokio::test]
async fn a_refund_failing_through_its_status_update_restores_the_credit() -> TestResult {
    use meteroid_store::repositories::payment_transactions::{
        RefundSettlement, RefundSettlementStatus,
    };
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "refund fails later"),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    let spend = draft(&setup, vec![line("Seats", 2_000, false)]).await?;
    finalize(&setup, spend.id).await?;

    let pending = ("re_fails_later", ObservedRefundStatus::Pending);
    provider_refund(&setup, receipt.transaction.id, 5_000, pending).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    assert_eq!(
        unrecovered_notices(&webhook_events(&setup).await?, wallet),
        vec![(CreditUnrecoveredCause::PaymentRefunded, 3_000, 2_000)]
    );

    let refund: meteroid_store::domain::PaymentTransaction =
        refunds_of(&setup, receipt.transaction.id)
            .await?
            .remove(0)
            .into();
    let store = setup.store.clone();
    setup
        .store
        .transaction(|conn| {
            async move {
                store
                    .settle_refund_tx(
                        conn,
                        &Actor::System,
                        refund,
                        RefundSettlement {
                            status: RefundSettlementStatus::Failed {
                                error: "insufficient balance".to_owned(),
                                external_refund_id: Some("re_fails_later".to_owned()),
                            },
                        },
                    )
                    .await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 3_000);
    assert_eq!(
        recovered_notices(&webhook_events(&setup).await?, wallet),
        vec![(CreditUnrecoveredCause::PaymentRefunded, 2_000, 0)]
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A refund in flight takes back a top-up whose credit then expires; the refund fails after that.
/// The money is back with the merchant and the credit returns to its grant, which has lapsed: the
/// expiry sweep books it as expired, so nothing becomes spendable after its expiry.
#[tokio::test]
async fn a_refund_failing_after_its_credit_expired_returns_it_as_expired() -> TestResult {
    let setup = setup().await;
    let wallet = wallet(&setup, CUST_UBER_ID).await?;
    let receipt = setup
        .services
        .top_up_wallet(
            top_up(CUST_UBER_ID, wallet, 5_000, "refund fails after expiry"),
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;

    let pending = ("re_after_expiry", ObservedRefundStatus::Pending);
    provider_refund(&setup, receipt.transaction.id, 5_000, pending).await?;
    drain(&setup).await;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    execute(
        &setup,
        format!(
            "UPDATE customer_credit_grant SET expires_at = now() + interval '1 second'
             WHERE tenant_id = '{}' AND invoice_id = '{}'",
            TENANT_ID.as_uuid(),
            receipt.invoice_id.as_uuid()
        ),
    )
    .await?;
    // Past the expiry set just above, whatever the load: the refund then fails after it.
    tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;

    let failed = ("re_after_expiry", ObservedRefundStatus::Failed);
    provider_refund(&setup, receipt.transaction.id, 0, failed).await?;
    drain(&setup).await;
    assert_eq!(
        balance(&setup, CUST_UBER_ID).await,
        0,
        "expired credit is not spendable"
    );
    setup
        .store
        .expire_customer_credit_for(TENANT_ID, CUST_UBER_ID, chrono::Utc::now())
        .await
        .map_err(test_error)?;
    assert_eq!(balance(&setup, CUST_UBER_ID).await, 0);
    let reasons = ledger_reasons(&setup, wallet).await?;
    assert!(
        reasons
            .iter()
            .any(|entry| entry.starts_with("Reinstatement 5000")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|entry| entry.starts_with("Expiry -5000")),
        "{reasons:?}"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    crate::helpers::invariants::assert_credit_invariants(&setup.store, TENANT_ID).await;
    Ok(())
}
