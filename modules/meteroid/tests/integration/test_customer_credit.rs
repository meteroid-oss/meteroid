//! Invoice-time customer credit, settled in the billing database beside its documents: finalize
//! draws, voids and credit notes return or take back value synchronously and exactly once.
use crate::data::ids::{CUST_SPOTIFY_ID, CUST_UBER_ID, TENANT_ID, USER_ID};
use crate::helpers::credit::{
    Grant, assert_ledger_balances, available, expire_grant, fund, fund_grant, grant_remaining,
};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, InvoiceId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::sql_types::{BigInt, Text, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::domain::credit::{
    CreditApplication, CreditGrant, CreditGrantSource, CreditGrantState, GrantClassification,
    LedgerEntryKind,
};
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::wallet_events::CreditUnrecoveredCause;
use meteroid_store::domain::{DetailedInvoice, Invoice, LineItem, LineItemKind};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::DebitCustomerCreditGrant;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use meteroid_store::services::SellCustomerCredit;
use rust_decimal::Decimal;
use std::sync::Arc;
use uuid::Uuid;

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

/// The customer's own currency: every document and grant of these tests is in it.
const EUR: Option<&str> = None;

async fn eur(setup: &MeteroidSetup) -> i64 {
    available(&setup.store, TENANT_ID, CUST_UBER_ID, EUR).await
}

pub(crate) use crate::helpers::invoice_lines::line;

pub(crate) use crate::helpers::invoices::{draft, finalize, pay_amount};

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

/// The upgrade freezes the shipped balance tables; a historical deduction is seeded as the previous
/// release left it.
async fn unfreeze_balance_tx(conn: &mut diesel_async::AsyncPgConnection) -> TestResult {
    use diesel_async::SimpleAsyncConnection;
    conn.batch_execute("ALTER TABLE customer_balance_tx DISABLE TRIGGER customer_balance_tx_frozen")
        .await
        .map_err(test_error)
}

async fn refreeze_balance_tx(conn: &mut diesel_async::AsyncPgConnection) -> TestResult {
    use diesel_async::SimpleAsyncConnection;
    conn.batch_execute("ALTER TABLE customer_balance_tx ENABLE TRIGGER customer_balance_tx_frozen")
        .await
        .map_err(test_error)
}

/// Settles an invoice by hand, with no payment: only for invoices whose payment is not under test.
async fn mark_paid(setup: &MeteroidSetup, invoice: InvoiceId) -> TestResult {
    execute(
        setup,
        format!(
            "UPDATE invoice SET amount_due=0, payment_status='PAID' WHERE tenant_id='{}' AND id='{}'",
            TENANT_ID.as_uuid(),
            invoice.as_uuid()
        ),
    )
    .await
}

/// Records a manual payment of the whole amount due and runs its settlement.
async fn pay(setup: &MeteroidSetup, invoice: &Invoice) -> TestResult {
    pay_amount(setup, invoice.id, invoice.amount_due).await
}

fn note(
    invoice: InvoiceId,
    credit_type: CreditType,
    lines: Vec<CreditLineItem>,
) -> CreateCreditNoteParams {
    CreateCreditNoteParams {
        invoice_id: invoice,
        line_items: lines,
        reason: Some("Customer credit test".into()),
        memo: None,
        credit_type,
        custom_properties: None,
    }
}

async fn issue(
    setup: &MeteroidSetup,
    params: CreateCreditNoteParams,
) -> meteroid_store::StoreResult<meteroid_store::domain::CreditNote> {
    setup
        .store
        .create_and_finalize_credit_note(Actor::System, TENANT_ID, params)
        .await
}

#[tokio::test]
async fn grants_draw_by_priority_and_expiry_and_usage_credit_pays_usage_only() -> TestResult {
    let setup = setup().await;
    let grant = |application, amount_minor, priority, expires_at| Grant {
        application,
        amount_minor,
        priority,
        expires_at,
    };
    let store = &setup.store;
    let late = fund_grant(
        store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        grant(CreditApplication::Invoice, 4_000, 100, None),
    )
    .await;
    let first = fund_grant(
        store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        grant(CreditApplication::Invoice, 1_000, 10, None),
    )
    .await;
    let usage = fund_grant(
        store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        grant(
            CreditApplication::Usage,
            9_000,
            100,
            Some(Utc::now() + chrono::Duration::days(30)),
        ),
    )
    .await;
    let invoice = draft(
        &setup,
        vec![line("Seats", 4_000, false), line("Usage", 3_000, true)],
    )
    .await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    // Usage credit covers only the 3000 of usage; invoice credit covers the fixed 4000, best
    // priority first.
    assert_eq!(finalized.applied_credits, 7_000);
    assert_eq!(finalized.amount_due, 0);
    let remaining = |id| grant_remaining(store, TENANT_ID, CUST_UBER_ID, id);
    assert_eq!(remaining(usage).await, 6_000);
    assert_eq!(remaining(first).await, 0);
    assert_eq!(remaining(late).await, 1_000);

    // Finalizing again is a no-op: the invoice drew once.
    finalize(&setup, invoice.id).await?;
    assert_eq!(remaining(late).await, 1_000);
    assert_ledger_balances(store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_draft_previews_the_credit_it_will_draw_and_finalize_draws_it() -> TestResult {
    let setup = setup().await;
    fund(&setup.store, TENANT_ID, CUST_UBER_ID, EUR, 2_500).await;
    let invoice = draft(&setup, vec![line("Seats", 4_000, false)]).await?;
    // An operator-edited draft keeps its lines on refresh; only the credit it would draw moves.
    execute(
        &setup,
        format!(
            "UPDATE invoice SET manually_edited_at=now() WHERE tenant_id='{}' AND id='{}'",
            TENANT_ID.as_uuid(),
            invoice.id.as_uuid()
        ),
    )
    .await?;
    let refreshed = setup
        .services
        .refresh_invoice_data(invoice.id, TENANT_ID)
        .await
        .map_err(test_error)?;
    assert_eq!(refreshed.invoice.applied_credits, 2_500);
    assert_eq!(refreshed.invoice.amount_due, 1_500);
    assert_eq!(eur(&setup).await, 2_500, "a preview draws nothing");

    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 2_500);
    assert_eq!(finalized.amount_due, 1_500);
    assert_eq!(eur(&setup).await, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn negative_total_invoice_becomes_customer_credit_once() -> TestResult {
    let setup = setup().await;
    let invoice = draft(&setup, vec![line("Downgrade proration", -2_500, false)]).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.status, InvoiceStatusEnum::Finalized);
    assert_eq!(finalized.total, -2_500);
    assert_eq!(finalized.applied_credits, 0);
    assert_eq!(eur(&setup).await, 2_500);
    finalize(&setup, invoice.id).await?;
    assert_eq!(
        eur(&setup).await,
        2_500,
        "the credit is funded exactly once"
    );
    let wallets = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    let grant = &wallets[0].grants[0];
    assert_eq!(grant.source, CreditGrantSource::NegativeInvoice);
    assert_eq!(grant.invoice_id, Some(invoice.id));
    assert_eq!(grant.classification, GrantClassification::Paid);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_voided_invoice_returns_the_credit_it_drew_once() -> TestResult {
    let setup = setup().await;
    let grant = fund(&setup.store, TENANT_ID, CUST_UBER_ID, EUR, 3_000).await;
    let invoice = draft(&setup, vec![line("Seats", 5_000, false)]).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 3_000);
    assert_eq!(eur(&setup).await, 0);

    let voided = setup
        .store
        .void_invoice(Actor::System, invoice.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(voided.status, InvoiceStatusEnum::Void);
    assert_eq!(eur(&setup).await, 3_000, "the drawn credit is back");
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, grant).await,
        3_000,
        "into the grant it came from"
    );
    // Nothing was collected, so the void's credit note returns no further value.
    let notes = setup
        .store
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].credited_amount_cents, 0);
    assert_eq!(notes[0].debt_cancelled_amount_cents, 0);
    assert!(
        setup
            .store
            .void_invoice(Actor::System, invoice.id, TENANT_ID)
            .await
            .is_err(),
        "a void is not repeated"
    );
    assert_eq!(eur(&setup).await, 3_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A shipped (pre-ledger) invoice: its applied credit was taken from the scalar in USD while the
/// invoice is in EUR. A void returns the original USD deduction, never converted invoice credit.
#[tokio::test]
async fn historical_invoice_void_restores_the_original_currency_once() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(&["I3"], 
        "seeds pre-ledger balances and purchases by raw SQL, without the cost the cutover backfills",
    );
    let invoice = draft(&setup, vec![line("Usage", 7_000, true)]).await?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::sql_query(
        "UPDATE invoice SET status='FINALIZED',invoice_number='HISTORICAL-FX-VOID',
        finalized_at=now(),applied_credits=3000,amount_due=4000 WHERE tenant_id=$1 AND id=$2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    unfreeze_balance_tx(&mut conn).await?;
    diesel::sql_query(
        "INSERT INTO customer_balance_tx (id,amount_cents,balance_cents_after,invoice_id,tenant_id,customer_id,currency)
         VALUES($1,$2,0,$3,$4,$5,$6)",
    )
    .bind::<SqlUuid, _>(Uuid::new_v4())
    .bind::<BigInt, _>(-5_000_i64)
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(CUST_UBER_ID.as_uuid())
    .bind::<Text, _>("USD")
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    refreeze_balance_tx(&mut conn).await?;
    drop(conn);
    let voided = setup
        .store
        .void_invoice(Actor::System, invoice.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(voided.status, InvoiceStatusEnum::Void);
    let usd = || available(&setup.store, TENANT_ID, CUST_UBER_ID, Some("USD"));
    assert_eq!(usd().await, 5_000, "the original USD deduction returns");
    assert_eq!(eur(&setup).await, 0, "never converted into invoice credit");
    assert!(
        setup
            .store
            .void_invoice(Actor::System, invoice.id, TENANT_ID)
            .await
            .is_err()
    );
    assert_eq!(usd().await, 5_000, "exactly once");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// m3. Voiding a pre-ledger invoice restores the EUR 50.00 it took from the old balance. That
/// value was the customer's before the release, like the balance carried over: it pays usage of
/// any date, here a usage line of January 2024 finalized now.
#[tokio::test]
async fn credit_a_void_of_a_pre_ledger_invoice_restored_pays_usage_of_any_date() -> TestResult {
    use diesel_models::customer_credit::CustomerCreditEntryRow;
    let setup = setup().await;
    let invoice = draft(&setup, vec![line("Seats", 7_000, false)]).await?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::sql_query(
        "UPDATE invoice SET status='FINALIZED',invoice_number='HISTORICAL-VOID-DATED',
        finalized_at=now(),applied_credits=5000,amount_due=2000 WHERE tenant_id=$1 AND id=$2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    unfreeze_balance_tx(&mut conn).await?;
    diesel::sql_query(
        "INSERT INTO customer_balance_tx (id,amount_cents,balance_cents_after,invoice_id,tenant_id,customer_id,currency)
         VALUES($1,$2,0,$3,$4,$5,'EUR')",
    )
    .bind::<SqlUuid, _>(Uuid::new_v4())
    .bind::<BigInt, _>(-5_000_i64)
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(CUST_UBER_ID.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    refreeze_balance_tx(&mut conn).await?;
    drop(conn);
    setup
        .store
        .void_invoice(Actor::System, invoice.id, TENANT_ID)
        .await
        .map_err(test_error)?;
    let restored = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .find(|grant| grant.source == CreditGrantSource::InvoiceVoid)
        .ok_or("the restored credit")?;
    assert!(restored.effective_at > Utc::now() - chrono::Duration::hours(1));

    let january = LineItem {
        start_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1).ok_or("a day")?,
        end_date: chrono::NaiveDate::from_ymd_opt(2024, 2, 1).ok_or("a day")?,
        ..line("Usage", 3_000, true)
    };
    let usage = draft(&setup, vec![january]).await?;
    let usage = finalize(&setup, usage.id).await?.invoice;
    assert_eq!(
        (usage.total, usage.applied_credits, usage.amount_due),
        (3_000, 3_000, 0)
    );
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let moved = CustomerCreditEntryRow::list_by_grant(&mut conn, TENANT_ID, restored.id)
        .await
        .map_err(test_error)?
        .into_iter()
        .map(|entry| format!("{} {}", entry.kind, entry.amount))
        .collect::<Vec<_>>();
    assert_eq!(moved, vec!["FUND 5000", "DRAW -3000"]);
    Ok(())
}

async fn historical_paid_invoice(setup: &MeteroidSetup, number: &str) -> TestResult<Invoice> {
    historical_paid_invoice_of(setup, number, 1_001).await
}

async fn historical_paid_invoice_of(
    setup: &MeteroidSetup,
    number: &str,
    deducted: i64,
) -> TestResult<Invoice> {
    let invoice = draft(setup, vec![line("Usage", 9_000, true)]).await?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::sql_query(
        "UPDATE invoice SET status='FINALIZED',invoice_number=$3,finalized_at=now(),
        applied_credits=3000,amount_due=6000 WHERE tenant_id=$1 AND id=$2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .bind::<Text, _>(number)
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    unfreeze_balance_tx(&mut conn).await?;
    diesel::sql_query(
        "INSERT INTO customer_balance_tx (id,amount_cents,balance_cents_after,invoice_id,tenant_id,customer_id,currency)
         VALUES($1,$5,0,$2,$3,$4,'USD')",
    )
    .bind::<SqlUuid, _>(Uuid::new_v4())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(CUST_UBER_ID.as_uuid())
    .bind::<BigInt, _>(-deducted)
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    refreeze_balance_tx(&mut conn).await?;
    drop(conn);
    // The cash share was collected; the credit share came from the scalar.
    pay_amount(setup, invoice.id, 6_000).await?;
    Ok(invoice)
}

#[tokio::test]
async fn historical_partial_refunds_return_the_original_currency_cumulatively() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(&["I3"], 
        "seeds pre-ledger balances and purchases by raw SQL, without the cost the cutover backfills",
    );
    let invoice = historical_paid_invoice(&setup, "HISTORICAL-PARTIAL-FX").await?;
    let params = note(
        invoice.id,
        CreditType::Refund,
        vec![CreditLineItem::Amount {
            local_id: invoice.line_items[0].local_id.clone(),
            amount_cents: 3_000,
        }],
    );
    let usd = || available(&setup.store, TENANT_ID, CUST_UBER_ID, Some("USD"));
    let mut notes = Vec::new();
    for cumulative in [334, 667, 1_001] {
        let settled = issue(&setup, params.clone()).await.map_err(test_error)?;
        assert_eq!(
            settled.credited_amount_cents, 1_000,
            "the note stays denominated in the invoice currency"
        );
        assert_eq!(settled.refunded_amount_cents, 2_000);
        assert_eq!(usd().await, cumulative);
        assert!(
            setup
                .store
                .finalize_credit_note(Actor::System, TENANT_ID, settled.id)
                .await
                .is_err(),
            "an issued note cannot be finalized again"
        );
        assert_eq!(usd().await, cumulative, "a replay credits nothing more");
        notes.push(settled.id);
    }
    setup
        .store
        .void_credit_note(Actor::System, TENANT_ID, notes[1])
        .await
        .map_err(test_error)?;
    assert_eq!(usd().await, 668, "the void removes its exact USD grant");
    issue(&setup, params).await.map_err(test_error)?;
    assert_eq!(usd().await, 1_001);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn historical_refund_after_an_unrecorded_return_is_refused() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(&["I3"], 
        "seeds pre-ledger balances and purchases by raw SQL, without the cost the cutover backfills",
    );
    let invoice = historical_paid_invoice(&setup, "HISTORICAL-AMBIGUOUS").await?;
    let params = note(
        invoice.id,
        CreditType::Refund,
        vec![CreditLineItem::Amount {
            local_id: invoice.line_items[0].local_id.clone(),
            amount_cents: 3_000,
        }],
    );
    let legacy = setup
        .store
        .create_credit_note(Actor::System, TENANT_ID, params.clone())
        .await
        .map_err(test_error)?;
    // Finalized on main: its return went through the converted scalar, with no ledger record.
    execute(
        &setup,
        format!(
            "UPDATE credit_note SET status='FINALIZED',credit_note_number='LEGACY-REFUND',finalized_at=now()
             WHERE tenant_id='{t}' AND id='{n}';
             UPDATE credit_note SET credited_retired_balance = true WHERE tenant_id = '{t}' AND id = '{n}'",
            t = TENANT_ID.as_uuid(),
            n = legacy.id.as_uuid()
        ),
    )
    .await?;
    let error = issue(&setup, params)
        .await
        .expect_err("an inexact return is refused");
    assert!(
        matches!(error.current_context(), StoreError::FailedPrecondition(message)
            if message.contains("no original-currency restoration record")),
        "{error:?}"
    );
    assert_eq!(
        available(&setup.store, TENANT_ID, CUST_UBER_ID, Some("USD")).await,
        0
    );
    Ok(())
}

/// A refund whose share rounds to nothing returns no credit, and must not block the next one.
#[tokio::test]
async fn historical_refunds_that_round_to_nothing_do_not_block_later_ones() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(&["I3"], 
        "seeds pre-ledger balances and purchases by raw SQL, without the cost the cutover backfills",
    );
    let invoice = historical_paid_invoice_of(&setup, "HISTORICAL-ROUNDING", 1).await?;
    let params = note(
        invoice.id,
        CreditType::Refund,
        vec![CreditLineItem::Amount {
            local_id: invoice.line_items[0].local_id.clone(),
            amount_cents: 3_000,
        }],
    );
    let usd = || available(&setup.store, TENANT_ID, CUST_UBER_ID, Some("USD"));
    for cumulative in [0, 1, 1] {
        issue(&setup, params.clone()).await.map_err(test_error)?;
        assert_eq!(usd().await, cumulative);
    }
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A deduction stamped with the customer's own currency (what the ledger migration stamps every
/// pre-ledger row with) returns exactly, in that currency, with no conversion.
#[tokio::test]
async fn a_historical_deduction_in_the_customers_currency_is_returned_exactly() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(&["I3"], 
        "seeds pre-ledger balances and purchases by raw SQL, without the cost the cutover backfills",
    );
    let invoice = draft(&setup, vec![line("Usage", 9_000, true)]).await?;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::sql_query(
        "UPDATE invoice SET status='FINALIZED',invoice_number='HISTORICAL-OWN-CCY',finalized_at=now(),
        applied_credits=3000,amount_due=6000 WHERE tenant_id=$1 AND id=$2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    unfreeze_balance_tx(&mut conn).await?;
    diesel::sql_query(
        "INSERT INTO customer_balance_tx (id,amount_cents,balance_cents_after,invoice_id,tenant_id,customer_id,currency)
         VALUES($1,$2,0,$3,$4,$5,'EUR')",
    )
    .bind::<SqlUuid, _>(Uuid::new_v4())
    .bind::<BigInt, _>(-1_001_i64)
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(CUST_UBER_ID.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    refreeze_balance_tx(&mut conn).await?;
    drop(conn);
    pay_amount(&setup, invoice.id, 6_000).await?;
    let params = note(
        invoice.id,
        CreditType::Refund,
        vec![CreditLineItem::Amount {
            local_id: invoice.line_items[0].local_id.clone(),
            amount_cents: 9_000,
        }],
    );
    let settled = issue(&setup, params).await.map_err(test_error)?;
    assert_eq!(
        settled.credited_amount_cents, 3_000,
        "the whole line, and so the whole applied credit, is refunded"
    );
    assert_eq!(
        eur(&setup).await,
        1_001,
        "the exact EUR deduction returns, unconverted"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_credit_to_balance_note_returns_paid_value_and_its_void_takes_it_back() -> TestResult {
    let setup = setup().await;
    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    finalize(&setup, invoice.id).await?;
    mark_paid(&setup, invoice.id).await?;
    let returned = issue(
        &setup,
        note(invoice.id, CreditType::CreditToBalance, vec![]),
    )
    .await
    .map_err(test_error)?;
    assert_eq!(returned.credited_amount_cents, 3_000);
    assert_eq!(returned.debt_cancelled_amount_cents, 0);
    assert_eq!(eur(&setup).await, 3_000);

    setup
        .store
        .void_credit_note(Actor::System, TENANT_ID, returned.id)
        .await
        .map_err(test_error)?;
    assert_eq!(
        eur(&setup).await,
        0,
        "the void takes the unspent credit back"
    );

    // Once the returned credit pays another invoice, its note can no longer be voided.
    let again = issue(
        &setup,
        note(invoice.id, CreditType::CreditToBalance, vec![]),
    )
    .await
    .map_err(test_error)?;
    let spend = draft(&setup, vec![line("Seats", 1_000, false)]).await?;
    assert_eq!(
        finalize(&setup, spend.id).await?.invoice.applied_credits,
        1_000
    );
    let refused = setup
        .store
        .void_credit_note(Actor::System, TENANT_ID, again.id)
        .await
        .expect_err("spent credit cannot be taken back");
    assert!(matches!(
        refused.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_eq!(eur(&setup).await, 2_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_credit_to_balance_note_on_an_unpaid_invoice_cancels_its_debt() -> TestResult {
    let setup = setup().await;
    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    finalize(&setup, invoice.id).await?;
    // Half paid: the note cancels the unpaid 1500 first and returns only collected cash.
    execute(
        &setup,
        format!(
            "UPDATE invoice SET amount_due=1500, payment_status='PARTIALLY_PAID' WHERE tenant_id='{}' AND id='{}'",
            TENANT_ID.as_uuid(),
            invoice.id.as_uuid()
        ),
    )
    .await?;
    let credited = issue(
        &setup,
        note(invoice.id, CreditType::CreditToBalance, vec![]),
    )
    .await
    .map_err(test_error)?;
    assert_eq!(credited.debt_cancelled_amount_cents, 1_500);
    assert_eq!(credited.credited_amount_cents, 1_500);
    assert_eq!(eur(&setup).await, 1_500, "only the collected share returns");
    let invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(invoice.amount_due, 0, "the cancelled debt is not collected");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A draft taken while the purchase was unpaid cancelled debt; paying it first makes the same
/// note a return of value, which the purchase's own grant already covers.
#[tokio::test]
async fn a_draft_credit_to_balance_note_cannot_double_mint_a_purchase_paid_meanwhile() -> TestResult
{
    let setup = setup().await;
    let invoice = buy(&setup, 10_000).await?;
    let set_payment_status = |status: &str| {
        execute(
            &setup,
            format!(
                "UPDATE invoice SET payment_status='{status}' WHERE tenant_id='{}' AND id='{}'",
                TENANT_ID.as_uuid(),
                invoice.id.as_uuid()
            ),
        )
    };
    // While its payment is in flight the purchase is not the customer's: no note on it at all.
    set_payment_status("PROCESSING").await?;
    let in_flight = setup
        .store
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            note(invoice.id, CreditType::CreditToBalance, vec![]),
        )
        .await
        .expect_err("the purchase is not funded while its payment is in flight");
    assert!(
        format!("{in_flight:?}").contains("wait until the payment settles"),
        "{in_flight:?}"
    );
    // Partly paid, a draft may return what was received; the purchase is then paid in full.
    set_payment_status("PARTIALLY_PAID").await?;
    let draft_note = setup
        .store
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            note(invoice.id, CreditType::CreditToBalance, vec![]),
        )
        .await
        .map_err(test_error)?;
    set_payment_status("UNPAID").await?;
    pay_amount(&setup, invoice.id, 10_000).await?;
    assert_eq!(eur(&setup).await, 10_000);

    let refused = setup
        .store
        .finalize_credit_note(Actor::System, TENANT_ID, draft_note.id)
        .await
        .expect_err("the purchase's credit is already granted");
    assert!(
        matches!(refused.current_context(), StoreError::FailedPrecondition(message)
            if message.contains("issue a Refund")),
        "{refused:?}"
    );
    assert_eq!(eur(&setup).await, 10_000);
    assert_eq!(
        grants_of(&setup, CreditGrantSource::Purchase).await?.len(),
        1
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The cutover leaves an unpaid purchase with its full face on the line: the mint nets the
/// cancelled debt once, so paying the rest funds exactly what was paid for.
#[tokio::test]
async fn a_carried_unpaid_purchase_with_cancelled_debt_funds_the_paid_remainder() -> TestResult {
    let setup = setup().await;
    let carried = LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(6_000),
        taxed_at_purchase: true,
        ..line("Purchase credits", 6_000, false)
    };
    let invoice = draft(&setup, vec![carried]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    issue(
        &setup,
        note(
            invoice.id,
            CreditType::DebtCancellation,
            by_amount(&invoice, 1_000),
        ),
    )
    .await
    .map_err(test_error)?;
    pay_amount(&setup, invoice.id, 5_000).await?;
    assert_eq!(eur(&setup).await, 5_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The expiry sweep records unspent value that lapsed as one EXPIRY entry; the next pass finds
/// nothing left to expire.
#[tokio::test]
async fn expiry_sweep_records_lapsed_credit_once() -> TestResult {
    let setup = setup().await;
    let grant = fund_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        Grant {
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            ..Grant::invoice(3_000)
        },
    )
    .await;
    fund(&setup.store, TENANT_ID, CUST_UBER_ID, EUR, 1_000).await;
    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        0,
        "live credit never lapses"
    );
    expire_grant(&setup.store, grant).await;
    assert_eq!(eur(&setup).await, 1_000, "lapsed value is not spendable");
    // A draft never draws lapsed value, even before the sweep records it.
    let invoice = draft(&setup, vec![line("Seats", 5_000, false)]).await?;
    assert_eq!(
        finalize(&setup, invoice.id).await?.invoice.applied_credits,
        1_000
    );
    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        1
    );
    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        0,
        "a second pass records nothing"
    );
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, grant).await,
        3_000,
        "the grant still shows what lapsed"
    );
    let wallet = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .remove(0);
    let expired = wallet.grants.iter().find(|g| g.id == grant).unwrap();
    assert_eq!(expired.state, CreditGrantState::Expired);
    assert_eq!(expired.expired_minor, Some(3_000));
    let (entries, _) = setup
        .store
        .customer_credit_ledger(TENANT_ID, wallet.id, 100, 0)
        .await
        .map_err(test_error)?;
    let expiries: Vec<i64> = entries
        .iter()
        .filter(|entry| entry.kind == LedgerEntryKind::Expiry)
        .map(|entry| entry.amount_minor)
        .collect();
    assert_eq!(expiries, vec![-3_000]);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Funds a Paid-classified grant directly: the shared `fund_grant` test helper always funds a
/// Promotional one, so burn-order tests that need both classifications in one wallet call the
/// store directly instead.
async fn fund_paid_grant(
    store: &meteroid_store::Store,
    tenant_id: common_domain::ids::TenantId,
    customer_id: common_domain::ids::CustomerId,
    currency: &str,
    amount_minor: i64,
    priority: i32,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> common_domain::ids::CreditGrantId {
    use meteroid_store::domain::credit::GrantClassification;
    use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
    let wallet = store
        .create_customer_credit_wallet(tenant_id, customer_id, currency)
        .await
        .expect("open customer credit wallet");
    store
        .fund_customer_credit_wallet(FundCustomerCreditWallet {
            tenant_id,
            customer_id,
            wallet_id: wallet.id,
            amount_minor,
            classification: GrantClassification::Paid,
            priority,
            expires_at,
            idempotency_key: Uuid::now_v7(),
            reason: "Integration-test paid credit".to_owned(),
            reference: None,
            actor_id: None,
            cost: None,
        })
        .await
        .expect("fund paid credit")
        .grant_id
}

/// At equal priority and expiry, the promotional grant burns before the paid one: priority and
/// expiry decide first, and classification is only the tie-break between otherwise-equal grants.
#[tokio::test]
async fn a_promotional_grant_burns_before_a_paid_one_at_equal_priority_and_expiry() -> TestResult {
    let setup = setup().await;
    let expires = Utc::now() + chrono::Duration::days(30);
    let promo = fund_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        Grant {
            expires_at: Some(expires),
            ..Grant::invoice(2_000)
        },
    )
    .await;
    let currency = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?
        .currency;
    let paid = fund_paid_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        &currency,
        5_000,
        100,
        Some(expires),
    )
    .await;

    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 3_000);
    assert_eq!(finalized.amount_due, 0);
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, promo).await,
        0,
        "the promotional grant is drawn first and fully, at equal priority and expiry"
    );
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, paid).await,
        4_000,
        "the paid grant pays only what the promo could not"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// When priority or expiry are NOT equal, that difference decides, overriding classification: a
/// paid grant expiring sooner than a promotional one burns first despite being paid, not promo.
#[tokio::test]
async fn a_sooner_expiring_paid_grant_burns_before_a_promotional_one_at_equal_priority()
-> TestResult {
    let setup = setup().await;
    let currency = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?
        .currency;
    let soon_paid = fund_paid_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        &currency,
        5_000,
        100,
        Some(Utc::now() + chrono::Duration::days(5)),
    )
    .await;
    let promo = fund_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        Grant {
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
            ..Grant::invoice(2_000)
        },
    )
    .await;

    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 3_000);
    assert_eq!(finalized.amount_due, 0);
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, soon_paid).await,
        2_000,
        "the sooner-expiring paid grant is drawn first, despite being paid rather than promotional"
    );
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, promo).await,
        2_000,
        "untouched: its later expiry ranks it after the paid grant"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The sweep writes one Expiry ledger entry per lapsed grant, and publishes `credit_grant.expired`
/// from that same write; a retried sweep that settles and lapses nothing more must not duplicate it.
#[tokio::test]
async fn credit_grant_expired_webhook_fires_once_even_when_the_sweep_is_retried() -> TestResult {
    let setup = setup().await;
    let grant = fund_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        Grant {
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            ..Grant::invoice(3_000)
        },
    )
    .await;
    expire_grant(&setup.store, grant).await;

    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        1
    );
    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        0,
        "a retried sweep settles and lapses nothing more"
    );

    let events = setup
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
        .filter(|event| matches!(event, OutboxEvent::CreditGrantExpired(e) if e.credit_grant_id == grant))
        .collect::<Vec<_>>();
    assert_eq!(
        events.len(),
        1,
        "the sweep's retry does not emit a second credit_grant.expired"
    );
    let OutboxEvent::CreditGrantExpired(expired) = &events[0] else {
        unreachable!()
    };
    assert_eq!(expired.expired_minor, 3_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A grant's idempotency key names one request of one customer: reused for another customer, or
/// with another amount, it is refused and grants nothing; the same request replays.
#[tokio::test]
async fn a_grant_key_reused_for_another_customer_or_request_is_refused() -> TestResult {
    use meteroid_store::domain::credit::GrantClassification;
    use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;

    let setup = setup().await;
    let key = Uuid::now_v7();
    let grant = |customer_id: common_domain::ids::CustomerId, amount_minor: i64| {
        let store = setup.store.clone();
        async move {
            let currency = store
                .find_customer_by_id(customer_id, TENANT_ID)
                .await?
                .currency;
            let wallet = store
                .create_customer_credit_wallet(TENANT_ID, customer_id, &currency)
                .await?;
            store
                .fund_customer_credit_wallet(FundCustomerCreditWallet {
                    tenant_id: TENANT_ID,
                    customer_id,
                    wallet_id: wallet.id,
                    amount_minor,
                    classification: GrantClassification::Promotional,
                    priority: 100,
                    expires_at: None,
                    idempotency_key: key,
                    reason: "Goodwill".to_owned(),
                    reference: None,
                    actor_id: None,
                    cost: None,
                })
                .await
        }
    };

    let first = grant(CUST_UBER_ID, 2_500).await.map_err(test_error)?;
    assert!(!first.replayed);
    for (customer_id, amount) in [(CUST_SPOTIFY_ID, 2_500), (CUST_UBER_ID, 9_000)] {
        let refused = grant(customer_id, amount)
            .await
            .expect_err("a reused key names another request");
        assert!(
            matches!(refused.current_context(), StoreError::Conflict(_)),
            "{refused:?}"
        );
    }
    assert_eq!(
        available(&setup.store, TENANT_ID, CUST_SPOTIFY_ID, None).await,
        0
    );
    let replay = grant(CUST_UBER_ID, 2_500).await.map_err(test_error)?;
    assert!(replay.replayed);
    assert_eq!(replay.grant_id, first.grant_id);
    assert_eq!(eur(&setup).await, 2_500);
    Ok(())
}

/// Credit added by hand can be partly removed, or removed for its unspent remainder, idempotently
/// per request; purchased or returned credit cannot, it is reversed by a credit note.
#[tokio::test]
async fn manual_credit_is_removed_exactly_and_idempotently() -> TestResult {
    let setup = setup().await;
    let grant = fund(&setup.store, TENANT_ID, CUST_UBER_ID, EUR, 5_000).await;
    let debit = |amount: Option<i64>, key: Uuid| DebitCustomerCreditGrant {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        grant_id: grant,
        amount_minor: amount,
        idempotency_key: key,
        reason: "Over-granted goodwill".to_owned(),
        actor_id: Some(USER_ID.as_uuid()),
    };
    let key = Uuid::now_v7();
    let first = setup
        .store
        .debit_customer_credit_grant(debit(Some(2_000), key))
        .await
        .map_err(test_error)?;
    assert_eq!((first.removed_minor, first.replayed), (2_000, false));
    let replay = setup
        .store
        .debit_customer_credit_grant(debit(Some(2_000), key))
        .await
        .map_err(test_error)?;
    assert_eq!((replay.removed_minor, replay.replayed), (2_000, true));
    let conflicting = setup
        .store
        .debit_customer_credit_grant(debit(Some(1_000), key))
        .await
        .expect_err("the same request cannot remove a different amount");
    assert!(matches!(
        conflicting.current_context(),
        StoreError::Conflict(_)
    ));
    let too_much = setup
        .store
        .debit_customer_credit_grant(debit(Some(3_001), Uuid::now_v7()))
        .await
        .expect_err("more than what is left");
    assert!(matches!(
        too_much.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_eq!(eur(&setup).await, 3_000);

    let removed = setup
        .store
        .debit_customer_credit_grant(debit(None, Uuid::now_v7()))
        .await
        .map_err(test_error)?;
    assert_eq!(removed.removed_minor, 3_000);
    assert_eq!(eur(&setup).await, 0);
    assert!(
        setup
            .store
            .debit_customer_credit_grant(debit(None, Uuid::now_v7()))
            .await
            .is_err(),
        "nothing is left to remove"
    );
    let (entries, _) = setup
        .store
        .customer_credit_ledger(TENANT_ID, removed.wallet.id, 100, 0)
        .await
        .map_err(test_error)?;
    let reversals: Vec<_> = entries
        .iter()
        .filter(|entry| entry.kind == LedgerEntryKind::Reversal)
        .map(|entry| (entry.amount_minor, entry.reason.as_str(), entry.created_by))
        .collect();
    assert_eq!(
        reversals,
        vec![
            (-3_000, "Over-granted goodwill", Some(USER_ID.as_uuid())),
            (-2_000, "Over-granted goodwill", Some(USER_ID.as_uuid())),
        ]
    );

    // Credit returned by a document is reversed by a credit note, never removed by hand.
    let invoice = draft(&setup, vec![line("Downgrade", -700, false)]).await?;
    finalize(&setup, invoice.id).await?;
    let returned = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .remove(0)
        .grants
        .into_iter()
        .find(|g| g.source == CreditGrantSource::NegativeInvoice)
        .expect("the negative invoice funded credit");
    let refused = setup
        .store
        .debit_customer_credit_grant(DebitCustomerCreditGrant {
            grant_id: returned.id,
            ..debit(None, Uuid::now_v7())
        })
        .await
        .expect_err("document credit is not removed by hand");
    assert!(matches!(
        refused.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A paid commitment: the purchase invoice carries the term, and its payment funds paid invoice
/// credit that expires that many months after payment. Credit never pays for a credit purchase.
#[tokio::test]
async fn a_paid_purchase_funds_invoice_credit_expiring_after_its_term() -> TestResult {
    let setup = setup().await;
    fund(&setup.store, TENANT_ID, CUST_UBER_ID, EUR, 900).await;
    let purchase = sell(&setup, 1_200_000, Some(12), Some("Annual commitment")).await?;
    let line = &purchase.invoice.line_items[0];
    assert_eq!(line.kind, LineItemKind::Credit);
    assert_eq!(line.credit_grant_amount, Some(1_200_000));
    assert_eq!(
        line.tax_amount, 0,
        "a currency prepayment is taxed when spent"
    );
    let finalized = purchase.invoice;
    assert_eq!(finalized.applied_credits, 0, "credit never buys credit");
    assert_eq!(finalized.amount_due, 1_200_000);
    assert_eq!(eur(&setup).await, 900, "nothing funds before payment");

    pay(&setup, &finalized).await?;
    let paid_at = Utc::now();
    let paid = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, finalized.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    let wallets = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    let grant = wallets
        .iter()
        .flat_map(|wallet| wallet.grants.iter())
        .find(|grant| grant.source == CreditGrantSource::Purchase)
        .expect("the paid purchase funds a grant");
    assert_eq!(grant.classification, GrantClassification::Paid);
    assert_eq!(grant.amount_minor, 1_200_000);
    assert_eq!(grant.invoice_id, Some(finalized.id));
    let expires_at = grant.expires_at.expect("a commitment expires");
    let term_end = paid_at
        .checked_add_months(chrono::Months::new(12))
        .expect("in range");
    assert!(
        (expires_at - term_end).num_minutes().abs() <= 5,
        "{expires_at} is twelve months after payment ({term_end})"
    );
    assert_eq!(eur(&setup).await, 1_200_900);

    // A second settlement of the same invoice funds nothing more.
    run_once_invoice_orchestration(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
    )
    .await;
    assert_eq!(eur(&setup).await, 1_200_900);

    // The expiring purchase is drawn before the non-expiring goodwill; a refund of the purchase
    // takes back only what is unspent of it.
    let spend = draft(&setup, vec![line_of(200_000)]).await?;
    assert_eq!(
        finalize(&setup, spend.id).await?.invoice.applied_credits,
        200_000
    );
    let refund = issue(&setup, note(finalized.id, CreditType::Refund, vec![]))
        .await
        .map_err(test_error)?;
    assert_eq!(refund.refunded_amount_cents, 1_000_000);
    assert_eq!(eur(&setup).await, 900, "the goodwill credit is untouched");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

fn line_of(cents: i64) -> LineItem {
    line("Seats", cents, false)
}

/// A plan version selling a pack (€50 for €60 of credit, twice, lapsing three months after
/// payment) and including €10 of credit each period at no charge.
async fn credit_pack_plan(setup: &MeteroidSetup) -> TestResult<common_domain::ids::PlanVersionId> {
    use common_domain::ids::{PlanId, PlanVersionId, PriceComponentId, ProductId};
    use diesel_async::AsyncConnection;
    use diesel_models::enums::{FeeTypeEnum, PlanStatusEnum, PlanTypeEnum};
    use diesel_models::errors::DatabaseErrorContainer;
    use diesel_models::plan_versions::PlanVersionRowNew;
    use diesel_models::plans::{PlanRowNew, PlanRowPatch};
    use diesel_models::price_components::PriceComponentRowNew;
    use diesel_models::products::ProductRowNew;
    use meteroid_store::domain::FeeType;
    use meteroid_store::domain::price_components::CreditExpiry;

    let plan_id = PlanId::new();
    let plan_version_id = PlanVersionId::new();
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    conn.transaction(async |tx| {
        PlanRowNew {
            id: plan_id,
            name: "Credit packs".to_string(),
            description: None,
            tenant_id: TENANT_ID,
            product_family_id: crate::data::ids::PRODUCT_FAMILY_ID,
            plan_type: PlanTypeEnum::Standard,
            status: PlanStatusEnum::Active,
        }
        .insert(tx)
        .await?;
        PlanVersionRowNew {
            id: plan_version_id,
            is_draft_version: false,
            plan_id,
            version: 1,
            trial_duration_days: None,
            tenant_id: TENANT_ID,
            period_start_day: None,
            net_terms: 0,
            currency: "EUR".to_string(),
            billing_cycles: None,
            trialing_plan_id: None,
            trial_is_free: false,
            uses_product_pricing: true,
            tax_inclusive: false,
            credit_exhausted_rule: None,
            credit_exhausted_caps: serde_json::json!({}),
        }
        .insert(tx)
        .await?;
        PlanRowPatch {
            id: plan_id,
            tenant_id: TENANT_ID,
            name: None,
            description: None,
            active_version_id: Some(Some(plan_version_id)),
            draft_version_id: None,
            self_service_rank: None,
            issues_license: None,
        }
        .update(tx)
        .await?;
        for (name, fee) in [
            (
                "Credit pack",
                FeeType::Credit {
                    unit_price: Decimal::new(50_00, 2),
                    quantity: 2,
                    credit_amount: Decimal::new(60_00, 2),
                    expiry: Some(CreditExpiry::Month(3)),
                    once: false,
                },
            ),
            (
                "Included credit",
                FeeType::Credit {
                    unit_price: Decimal::ZERO,
                    quantity: 1,
                    credit_amount: Decimal::new(10_00, 2),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    once: false,
                },
            ),
        ] {
            let product_id = ProductId::new();
            ProductRowNew {
                id: product_id,
                name: name.to_string(),
                description: None,
                tenant_id: TENANT_ID,
                product_family_id: crate::data::ids::PRODUCT_FAMILY_ID,
                fee_type: FeeTypeEnum::Credit,
                fee_structure: serde_json::to_value(
                    &meteroid_store::domain::prices::FeeStructure::Credit {},
                )
                .expect("fee structure serializes"),
                catalog: true,
                tax_category_id: None,
            }
            .insert(tx)
            .await?;
            PriceComponentRowNew {
                id: PriceComponentId::new(),
                name: name.to_string(),
                legacy_fee: Some(fee.try_into().expect("fee serializes")),
                plan_version_id,
                product_id: Some(product_id),
                billable_metric_id: None,
            }
            .insert(tx)
            .await?;
        }
        Ok::<(), DatabaseErrorContainer>(())
    })
    .await
    .map_err(test_error)?;
    Ok(plan_version_id)
}

#[tokio::test]
async fn a_plan_pack_funds_its_credit_when_paid_and_included_credit_when_invoiced() -> TestResult {
    use meteroid_store::domain::{
        CreateSubscription, PaginationRequest, SubscriptionActivationCondition, SubscriptionNew,
    };
    let setup = setup().await;
    let plan_version_id = credit_pack_plan(&setup).await?;
    let today = Utc::now().date_naive();
    let subscription = setup
        .services
        .insert_subscription(
            Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: today,
                    end_date: None,
                    billing_start_date: Some(today),
                    activation_condition: SubscriptionActivationCondition::OnStart,
                    trial_duration: None,
                    billing_day_anchor: None,
                    payment_methods_config: None,
                    auto_advance_invoices: true,
                    charge_automatically: false,
                    purchase_order: None,
                    backdate_invoices: false,
                    skip_checkout_session: false,
                    skip_past_invoices: false,
                    usage_filter: None,
                    custom_properties: serde_json::Value::Null,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .map_err(test_error)?;
    setup
        .services
        .get_and_process_due_events()
        .await
        .map_err(test_error)?;
    let invoices = setup
        .store
        .list_invoices(
            TENANT_ID,
            None,
            Some(subscription.id),
            None,
            None,
            None,
            false,
            None,
            vec![],
            Some("created_at.asc".to_string()),
            PaginationRequest {
                page: 0,
                per_page: None,
            },
        )
        .await
        .map_err(test_error)?
        .items;
    let invoice = invoices[0].invoice.clone();
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    let pack = invoice
        .line_items
        .iter()
        .find(|line| line.name.starts_with("Credit pack"))
        .expect("the pack is billed");
    assert_eq!(pack.kind, LineItemKind::Credit);
    assert_eq!(pack.amount_subtotal, 10_000, "two packs at €50");
    assert_eq!(pack.credit_grant_amount, Some(12_000), "two packs of €60");
    assert_eq!(pack.tax_amount, 0);
    assert!(!pack.credit_grant_included);
    let included = invoice
        .line_items
        .iter()
        .find(|line| line.name.starts_with("Included credit"))
        .expect("the included credit is listed");
    assert_eq!(included.amount_subtotal, 0);
    assert_eq!(included.credit_grant_amount, Some(10_00));
    assert!(included.credit_grant_included);
    assert_eq!(invoice.applied_credits, 0, "credit never buys credit");

    // Issuing the invoice funds the included credit for its period, as given credit.
    let wallet = |kind: CreditGrantSource| {
        let store = setup.store.clone();
        async move {
            store
                .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
                .await
                .expect("wallets")
                .into_iter()
                .flat_map(|wallet| wallet.grants)
                .filter(|grant| grant.source == kind)
                .collect::<Vec<_>>()
        }
    };
    let included_grants = wallet(CreditGrantSource::Included).await;
    assert_eq!(included_grants.len(), 1);
    assert_eq!(included_grants[0].amount_minor, 10_00);
    assert_eq!(
        included_grants[0].classification,
        GrantClassification::Promotional
    );
    assert_eq!(
        included_grants[0].expires_at,
        Some(included.end_date.and_time(chrono::NaiveTime::MIN).and_utc()),
        "included credit lapses with its period"
    );
    assert!(
        wallet(CreditGrantSource::Purchase).await.is_empty(),
        "unpaid packs fund nothing"
    );

    pay(&setup, &invoice).await?;
    // €100 buys €120: €100 of paid credit and €20 of bonus credit, with the same terms.
    let packs = wallet(CreditGrantSource::Purchase).await;
    assert_eq!(packs.len(), 2);
    let paid = packs
        .iter()
        .find(|grant| grant.classification == GrantClassification::Paid)
        .expect("the paid credit");
    let bonus = packs
        .iter()
        .find(|grant| grant.classification == GrantClassification::Promotional)
        .expect("the bonus credit");
    assert_eq!((paid.amount_minor, bonus.amount_minor), (10_000, 2_000));
    assert_eq!(paid.invoice_id, Some(invoice.id));
    assert_eq!(bonus.invoice_id, Some(invoice.id));
    assert!(paid.expires_at.is_some());
    assert_eq!(bonus.expires_at, paid.expires_at);
    assert_eq!(eur(&setup).await, 13_000);
    run_once_invoice_orchestration(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
    )
    .await;
    assert_eq!(eur(&setup).await, 13_000, "a pack funds once");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

async fn grants_of(setup: &MeteroidSetup, kind: CreditGrantSource) -> TestResult<Vec<CreditGrant>> {
    Ok(setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == kind)
        .collect())
}

/// Drawn credit a note returns goes back to the grants it came from with their terms (a
/// promotional, expiring grant stays so); only collected cash becomes new paid credit.
#[tokio::test]
async fn credit_notes_return_drawn_credit_to_its_grants_with_their_terms() -> TestResult {
    let setup = setup().await;
    let expires_at = Utc::now() + chrono::Duration::days(30);
    let promo = fund_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        EUR,
        Grant {
            expires_at: Some(expires_at),
            ..Grant::invoice(3_000)
        },
    )
    .await;
    let invoice = draft(&setup, vec![line("Seats", 5_000, false)]).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(finalized.applied_credits, 3_000);
    pay(&setup, &finalized).await?;
    let remaining = || grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, promo);

    let balance = issue(
        &setup,
        note(invoice.id, CreditType::CreditToBalance, vec![]),
    )
    .await
    .map_err(test_error)?;
    assert_eq!(balance.credited_amount_cents, 5_000);
    assert_eq!(
        remaining().await,
        3_000,
        "drawn credit returns to its grant"
    );
    let cash = grants_of(&setup, CreditGrantSource::CreditNote).await?;
    assert_eq!(cash.len(), 1);
    assert_eq!(
        cash[0].amount_minor, 2_000,
        "only the collected cash is new"
    );
    assert_eq!(cash[0].classification, GrantClassification::Paid);
    assert_eq!(cash[0].expires_at, None);
    assert_eq!(eur(&setup).await, 5_000);

    setup
        .store
        .void_credit_note(Actor::System, TENANT_ID, balance.id)
        .await
        .map_err(test_error)?;
    assert_eq!(remaining().await, 0, "the void applies the credit again");
    assert_eq!(eur(&setup).await, 0);

    let refund = issue(&setup, note(invoice.id, CreditType::Refund, vec![]))
        .await
        .map_err(test_error)?;
    assert_eq!(refund.credited_amount_cents, 3_000);
    assert_eq!(refund.refunded_amount_cents, 2_000);
    assert_eq!(remaining().await, 3_000);
    let wallets = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    let returned = wallets
        .iter()
        .flat_map(|wallet| wallet.grants.iter())
        .find(|grant| grant.id == promo)
        .expect("the promotional grant");
    assert_eq!(returned.classification, GrantClassification::Paid);
    assert_eq!(
        returned.expires_at.map(|at| at.timestamp_micros()),
        Some(expires_at.timestamp_micros()),
        "the refunded credit keeps its expiry"
    );
    assert_eq!(eur(&setup).await, 3_000, "no new grant for the refund");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

fn included_credit(amount: i64) -> LineItem {
    LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(amount),
        credit_grant_included: true,
        ..line("Included credit", 0, false)
    }
}

/// A voided invoice takes back the credit its issue included; refused while another invoice holds
/// any of it, allowed once that one is voided.
#[tokio::test]
async fn a_voided_invoice_takes_back_its_included_credit() -> TestResult {
    let setup = setup().await;
    let void = |id: InvoiceId| {
        let store = setup.store.clone();
        async move { store.void_invoice(Actor::System, id, TENANT_ID).await }
    };
    let unspent = draft(
        &setup,
        vec![included_credit(1_000), line("Seats", 2_000, false)],
    )
    .await?;
    finalize(&setup, unspent.id).await?;
    assert_eq!(eur(&setup).await, 1_000);
    void(unspent.id).await.map_err(test_error)?;
    assert_eq!(eur(&setup).await, 0, "unspent included credit goes with it");

    let granting = draft(
        &setup,
        vec![included_credit(1_000), line("Seats", 2_000, false)],
    )
    .await?;
    finalize(&setup, granting.id).await?;
    // Included credit pays its subscription's usage only.
    let spend = draft(&setup, vec![line("Usage", 400, true)]).await?;
    assert_eq!(
        finalize(&setup, spend.id)
            .await?
            .invoice
            .promotional_credits,
        400,
        "included credit is promotional, taken off before tax"
    );
    let refused = void(granting.id)
        .await
        .expect_err("spent included credit cannot be taken back");
    assert!(matches!(
        refused.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_eq!(eur(&setup).await, 600);
    void(spend.id).await.map_err(test_error)?;
    void(granting.id).await.map_err(test_error)?;
    assert_eq!(eur(&setup).await, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Included credit that rolls over in full when its period ends.
fn rolling_included_credit(amount: i64) -> LineItem {
    use meteroid_store::domain::credit::GrantRollover;
    use meteroid_store::domain::price_components::CreditExpiry;
    let included = included_credit(amount);
    LineItem {
        // Begun two days ago, so an expiry moved into the past still follows its start.
        start_date: included.start_date - chrono::Duration::days(2),
        credit_grant_expiry: Some(CreditExpiry::BillingPeriod),
        credit_grant_rollover: Some(GrantRollover {
            max_minor: None,
            expiry: CreditExpiry::Month(1),
        }),
        ..included
    }
}

/// Finalizes an invoice including rolling credit, then expires and sweeps it: the value is now
/// held by the grant it rolled over into, which no longer names the invoice.
async fn rolled_included_credit(setup: &MeteroidSetup, amount: i64) -> TestResult<InvoiceId> {
    let invoice = draft(
        setup,
        vec![rolling_included_credit(amount), line("Seats", 2_000, false)],
    )
    .await?;
    finalize(setup, invoice.id).await?;
    let included = grants_of(setup, CreditGrantSource::Included)
        .await?
        .into_iter()
        .find(|grant| grant.remaining_minor > 0)
        .expect("the included grant");
    expire_grant(&setup.store, included.id).await;
    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        1
    );
    assert_eq!(
        grants_of(setup, CreditGrantSource::Rollover)
            .await?
            .iter()
            .map(|grant| grant.remaining_minor)
            .sum::<i64>(),
        amount,
        "the included credit rolled over"
    );
    Ok(invoice.id)
}

/// Voiding the invoice takes back the credit its included credit rolled over into (old code only
/// looked at the invoice's own, now empty, grant and left the rolled value spendable). Spent rolled
/// credit refuses the void, exactly as spent included credit does.
#[tokio::test]
async fn a_voided_invoice_takes_back_the_credit_its_included_credit_rolled_into() -> TestResult {
    let setup = setup().await;
    let void = |id: InvoiceId| {
        let store = setup.store.clone();
        async move { store.void_invoice(Actor::System, id, TENANT_ID).await }
    };
    let unspent = rolled_included_credit(&setup, 1_000).await?;
    assert_eq!(eur(&setup).await, 1_000);
    void(unspent).await.map_err(test_error)?;
    assert_eq!(
        eur(&setup).await,
        0,
        "the rolled credit goes with the invoice"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;

    let granting = rolled_included_credit(&setup, 1_000).await?;
    // Included credit pays its subscription's usage only.
    let spend = draft(&setup, vec![line("Usage", 400, true)]).await?;
    assert_eq!(
        finalize(&setup, spend.id)
            .await?
            .invoice
            .promotional_credits,
        400,
        "included credit is promotional, taken off before tax"
    );
    let refused = void(granting)
        .await
        .expect_err("spent rolled credit cannot be taken back");
    assert!(matches!(
        refused.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_eq!(eur(&setup).await, 600);
    void(spend.id).await.map_err(test_error)?;
    void(granting).await.map_err(test_error)?;
    assert_eq!(eur(&setup).await, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A note finalized before the ledger credited the scalar, now carried in the CUSTOMER_BALANCE
/// grant: its void takes the value back from there, and is refused once that value is spent.
#[tokio::test]
async fn voiding_a_pre_ledger_note_takes_back_the_carried_balance() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(&["I7"], 
        "seeds pre-ledger balances and purchases by raw SQL, without the cost the cutover backfills",
    );
    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    finalize(&setup, invoice.id).await?;
    mark_paid(&setup, invoice.id).await?;
    let (tenant, customer, invoice_id) = (TENANT_ID.as_uuid(), CUST_UBER_ID.as_uuid(), invoice.id);
    execute(
        &setup,
        format!(
            "WITH a AS (INSERT INTO customer_credit_account (id, tenant_id, customer_id, currency)
                        VALUES (gen_random_uuid(), '{tenant}', '{customer}', 'EUR') RETURNING id),
                  g AS (INSERT INTO customer_credit_grant (id, application, tenant_id, customer_id, account_id, amount, remaining,
                                                           priority, classification, effective_at, source_kind,
                                                           source_id, reason, created_at)
                        SELECT gen_random_uuid(), 'INVOICE', '{tenant}', '{customer}', a.id, 5000, 5000, 100, 'PAID', now(),
                               'CUSTOMER_BALANCE', '{customer}', 'Customer balance carried over', now()
                        FROM a RETURNING id)
             INSERT INTO customer_credit_entry (id, tenant_id, customer_id, grant_id, kind, amount, source_id, created_at)
             SELECT gen_random_uuid(), '{tenant}', '{customer}', g.id, 'FUND', 5000, '{customer}', now() FROM g;"
        ),
    )
    .await?;
    let legacy = |number: &'static str| {
        let setup = &setup;
        async move {
            let created = setup
                .store
                .create_credit_note(
                    Actor::System,
                    TENANT_ID,
                    note(invoice_id, CreditType::CreditToBalance, vec![]),
                )
                .await
                .map_err(test_error)?;
            assert_eq!(created.credited_amount_cents, 3_000);
            execute(
                setup,
                format!(
                    "UPDATE credit_note SET status='FINALIZED',credit_note_number='{number}',finalized_at=now()
                     WHERE tenant_id='{tenant}' AND id='{id}';
                     UPDATE credit_note SET credited_retired_balance = true WHERE tenant_id = '{tenant}' AND id = '{id}';",
                    id = created.id.as_uuid()
                ),
            )
            .await?;
            Ok::<_, Box<dyn std::error::Error>>(created.id)
        }
    };
    let first = legacy("LEGACY-CTB-1").await?;
    assert_eq!(eur(&setup).await, 5_000);
    setup
        .store
        .void_credit_note(Actor::System, TENANT_ID, first)
        .await
        .map_err(test_error)?;
    assert_eq!(
        eur(&setup).await,
        2_000,
        "the note's 3000 leaves the balance"
    );

    let second = legacy("LEGACY-CTB-2").await?;
    let refused = setup
        .store
        .void_credit_note(Actor::System, TENANT_ID, second)
        .await
        .expect_err("only 2000 of the balance is left");
    assert!(matches!(
        refused.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    assert_eq!(eur(&setup).await, 2_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

async fn sell(
    setup: &MeteroidSetup,
    amount_minor: i64,
    expires_in_months: Option<u32>,
    note: Option<&str>,
) -> TestResult<DetailedInvoice> {
    let customer = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &customer.currency)
        .await
        .map_err(test_error)?;
    let sold = setup
        .services
        .sell_customer_credit(
            Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor,
                expires_in_months,
                note: note.map(str::to_owned),
                actor: Actor::User { id: USER_ID },
                unit_price: None,
                expected_unit_price: None,
            },
        )
        .await
        .map_err(test_error)?;
    Ok(sold.invoice)
}

async fn buy(setup: &MeteroidSetup, cents: i64) -> TestResult<Invoice> {
    Ok(sell(setup, cents, None, None).await?.invoice)
}

fn by_amount(invoice: &Invoice, amount_cents: i64) -> Vec<CreditLineItem> {
    vec![CreditLineItem::Amount {
        local_id: invoice.line_items[0].local_id.clone(),
        amount_cents,
    }]
}

/// Forgiven debt was never paid for: a purchase funds only the credit its payments bought, and
/// the note that shaped the grant can no longer be voided.
#[tokio::test]
async fn a_purchase_funds_only_the_credit_paid_for_net_of_forgiven_debt() -> TestResult {
    let setup = setup().await;
    let cancelled_first = buy(&setup, 10_000).await?;
    issue(
        &setup,
        note(
            cancelled_first.id,
            CreditType::DebtCancellation,
            by_amount(&cancelled_first, 5_000),
        ),
    )
    .await
    .map_err(test_error)?;
    pay_amount(&setup, cancelled_first.id, 5_000).await?;
    assert_eq!(eur(&setup).await, 5_000, "half cancelled, half paid");
    let grants = grants_of(&setup, CreditGrantSource::Purchase).await?;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].amount_minor, 5_000);

    let cancellation = setup
        .store
        .list_credit_notes_by_invoice_id(TENANT_ID, cancelled_first.id)
        .await
        .map_err(test_error)?
        .remove(0);
    assert!(
        setup
            .store
            .void_credit_note(Actor::System, TENANT_ID, cancellation.id)
            .await
            .is_err(),
        "the grant was funded net of the cancellation"
    );

    // Refunding the rest reverses exactly what was funded.
    let refund = issue(&setup, note(cancelled_first.id, CreditType::Refund, vec![]))
        .await
        .map_err(test_error)?;
    assert_eq!(refund.refunded_amount_cents, 5_000);
    assert_eq!(eur(&setup).await, 0);

    // Paid first, the rest forgiven: the forgiveness settles the invoice and funds the paid part.
    let paid_first = buy(&setup, 10_000).await?;
    // A partial collection, as a provider payment leaves it: manual payments settle in full.
    execute(
        &setup,
        format!(
            "UPDATE invoice SET amount_due=6000, payment_status='PARTIALLY_PAID' WHERE tenant_id='{}' AND id='{}'",
            TENANT_ID.as_uuid(),
            paid_first.id.as_uuid()
        ),
    )
    .await?;
    issue(
        &setup,
        note(
            paid_first.id,
            CreditType::DebtCancellation,
            by_amount(&paid_first, 6_000),
        ),
    )
    .await
    .map_err(test_error)?;
    let settled = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, paid_first.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(settled.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(eur(&setup).await, 4_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A line of 30 selling 10 units of credit, refunded in three equal notes: rounding is cumulative
/// so the full refund takes back the full grant.
#[tokio::test]
async fn partial_refunds_of_a_credit_line_reverse_its_whole_grant() -> TestResult {
    let setup = setup().await;
    setup.skip_checks(
        &["I7"],
        "sells a currency pack above its face through store internals, which no API allows",
    );
    let pack = LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(10),
        ..line("Credit pack", 30, false)
    };
    let invoice = draft(&setup, vec![pack]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    pay(&setup, &invoice).await?;
    assert_eq!(eur(&setup).await, 10);

    let mut reversed = Vec::new();
    for _ in 0..3 {
        issue(
            &setup,
            note(invoice.id, CreditType::Refund, by_amount(&invoice, 10)),
        )
        .await
        .map_err(test_error)?;
        reversed.push(10 - eur(&setup).await);
    }
    assert_eq!(reversed, [3, 7, 10]);
    assert_eq!(
        grants_of(&setup, CreditGrantSource::Purchase).await?[0].remaining_minor,
        0
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

async fn count(setup: &MeteroidSetup, sql: &str) -> TestResult<i64> {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let rows: Vec<Count> = diesel::sql_query(sql)
        .load(&mut conn)
        .await
        .map_err(test_error)?;
    Ok(rows[0].n)
}

async fn sweep(setup: &MeteroidSetup) -> TestResult<usize> {
    setup
        .store
        .reconcile_reversed_credit_purchases(100)
        .await
        .map_err(test_error)
}

async fn reversal_entries(setup: &MeteroidSetup) -> TestResult<(i64, i64)> {
    Ok((
        count(
            setup,
            "SELECT count(*)::bigint AS n FROM customer_credit_entry WHERE kind = 'REVERSAL'",
        )
        .await?,
        count(
            setup,
            "SELECT coalesce(sum(abs(amount)), 0)::bigint AS n FROM customer_credit_entry WHERE kind = 'REVERSAL'",
        )
        .await?,
    ))
}

async fn unrecovered_notices(
    setup: &MeteroidSetup,
) -> TestResult<Vec<(CreditUnrecoveredCause, i64, i64)>> {
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
            OutboxEvent::WalletCreditUnrecovered(event) => {
                Some((event.cause, event.taken_back_minor, event.unrecovered_minor))
            }
            _ => None,
        })
        .collect())
}

/// A purchase paid, then 1500 of its 5000 credit spent.
async fn paid_purchase_partly_spent(setup: &MeteroidSetup) -> TestResult<Invoice> {
    let purchase = buy(setup, 5_000).await?;
    pay(setup, &purchase).await?;
    assert_eq!(eur(setup).await, 5_000);
    let spend = draft(setup, vec![line("Seats", 1_500, false)]).await?;
    assert_eq!(
        finalize(setup, spend.id).await?.invoice.applied_credits,
        1_500
    );
    assert_eq!(eur(setup).await, 3_500);
    Ok(purchase)
}

/// The previous release reverses a payment by updating the payment and reopening the invoice, with
/// no credit movement: the sweep takes back the unspent share once.
#[tokio::test]
async fn the_sweep_reverses_the_credit_of_a_payment_reversed_by_the_previous_release() -> TestResult
{
    let setup = setup().await;
    let purchase = paid_purchase_partly_spent(&setup).await?;
    assert_eq!(sweep(&setup).await?, 0, "nothing reversed, nothing to do");

    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let reversed = diesel::sql_query(
        "UPDATE payment_transaction SET amount_reversed = amount, status = 'REFUNDED', reversal_kind = 'CHARGEBACK'
         WHERE tenant_id = $1 AND invoice_id = $2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(purchase.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    assert_eq!(reversed, 1);
    diesel::sql_query(
        "UPDATE invoice SET amount_due = total, payment_status = 'UNPAID' WHERE tenant_id = $1 AND id = $2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(purchase.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    drop(conn);
    assert_eq!(eur(&setup).await, 3_500, "still spendable before the sweep");

    assert_eq!(sweep(&setup).await?, 1);
    assert_eq!(eur(&setup).await, 0);
    assert_eq!(reversal_entries(&setup).await?, (1, 3_500));
    assert_eq!(
        unrecovered_notices(&setup).await?,
        vec![(CreditUnrecoveredCause::PaymentReversed, 3_500, 1_500)],
        "the 1500 already spent is reported"
    );

    assert_eq!(sweep(&setup).await?, 0, "a rerun is a no-op");
    assert_eq!(reversal_entries(&setup).await?, (1, 3_500));
    assert_eq!(unrecovered_notices(&setup).await?.len(), 1);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

async fn reverse_as_previous_release(setup: &MeteroidSetup, invoice: &Invoice) -> TestResult {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    diesel::sql_query(
        "UPDATE payment_transaction SET amount_reversed = amount, status = 'REFUNDED', reversal_kind = 'CHARGEBACK'
         WHERE tenant_id = $1 AND invoice_id = $2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    diesel::sql_query(
        "UPDATE invoice SET amount_due = total, payment_status = 'UNPAID' WHERE tenant_id = $1 AND id = $2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(invoice.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    Ok(())
}

/// One invoice per page: a reconciled purchase still fills its page, and the next one is reached.
#[tokio::test]
async fn the_reversal_sweep_pages_past_reconciled_purchases() -> TestResult {
    let setup = setup().await;
    let first = buy(&setup, 5_000).await?;
    pay(&setup, &first).await?;
    let second = buy(&setup, 2_000).await?;
    pay(&setup, &second).await?;
    assert_eq!(eur(&setup).await, 7_000);
    let sweep_by_one = || async {
        setup
            .store
            .reconcile_reversed_credit_purchases(1)
            .await
            .map_err(test_error)
    };

    reverse_as_previous_release(&setup, &first).await?;
    assert_eq!(sweep_by_one().await?, 1);
    assert_eq!(eur(&setup).await, 2_000);

    reverse_as_previous_release(&setup, &second).await?;
    assert_eq!(
        sweep_by_one().await?,
        1,
        "the reconciled purchase does not stop the page"
    );
    assert_eq!(eur(&setup).await, 0);
    assert_eq!(reversal_entries(&setup).await?, (2, 7_000));
    assert_eq!(sweep_by_one().await?, 0, "a rerun is a no-op");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// The previous release finalizes a cash refund note without touching the credit ledger: the sweep
/// takes back the unspent share once.
#[tokio::test]
async fn the_sweep_reverses_the_credit_of_a_refund_finalized_by_the_previous_release() -> TestResult
{
    let setup = setup().await;
    let purchase = buy(&setup, 5_000).await?;
    pay(&setup, &purchase).await?;
    let refund = setup
        .store
        .create_credit_note(
            Actor::System,
            TENANT_ID,
            note(purchase.id, CreditType::Refund, vec![]),
        )
        .await
        .map_err(test_error)?;
    assert_eq!(sweep(&setup).await?, 0, "a draft note moves nothing");
    let spend = draft(&setup, vec![line("Seats", 1_500, false)]).await?;
    assert_eq!(
        finalize(&setup, spend.id).await?.invoice.applied_credits,
        1_500
    );

    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let finalized = diesel::sql_query(
        "UPDATE credit_note SET status = 'FINALIZED', finalized_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind::<SqlUuid, _>(TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(refund.id.as_uuid())
    .execute(&mut conn)
    .await
    .map_err(test_error)?;
    assert_eq!(finalized, 1);
    drop(conn);
    assert_eq!(eur(&setup).await, 3_500, "still spendable before the sweep");

    assert_eq!(sweep(&setup).await?, 1);
    assert_eq!(eur(&setup).await, 0);
    assert_eq!(reversal_entries(&setup).await?, (1, 3_500));
    assert_eq!(
        unrecovered_notices(&setup).await?,
        vec![(CreditUnrecoveredCause::CreditNoteRefunded, 3_500, 1_500)],
        "the 1500 already spent is reported"
    );

    assert_eq!(sweep(&setup).await?, 0, "a rerun is a no-op");
    assert_eq!(reversal_entries(&setup).await?, (1, 3_500));
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Generates the invoice's PDF (a placeholder) and runs the orchestration it triggers.
async fn issue_pdf(setup: &MeteroidSetup, invoice: InvoiceId) -> TestResult {
    let pdf = setup
        .object_store
        .store(
            meteroid::services::storage::ObjectKind::InvoicePdf,
            TENANT_ID,
            bytes::Bytes::from_static(b"%PDF-1.4\n%%EOF\n"),
        )
        .await
        .map_err(test_error)?;
    setup
        .store
        .save_invoice_documents(invoice, TENANT_ID, CUST_UBER_ID, pdf, None, None)
        .await
        .map_err(test_error)?;
    for _ in 0..2 {
        run_once_invoice_orchestration(
            Arc::new(setup.store.clone()),
            Arc::new(setup.services.clone()),
        )
        .await;
    }
    Ok(())
}

/// A one-off invoice that credit covers in full has nothing to collect: once issued it is paid
/// like a subscription's zero invoice, `invoice.paid` fires once, and a credit-to-balance note
/// then gives the credit back. The old code left it unpaid, so every credit note was refused.
#[tokio::test]
async fn a_one_off_invoice_fully_covered_by_credit_is_paid_and_can_be_credited() -> TestResult {
    let setup = setup().await;
    let grant = fund(&setup.store, TENANT_ID, CUST_UBER_ID, EUR, 5_000).await;
    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    let finalized = finalize(&setup, invoice.id).await?.invoice;
    assert_eq!(
        (finalized.applied_credits, finalized.amount_due),
        (3_000, 0)
    );
    assert_eq!(finalized.payment_status, InvoicePaymentStatus::Unpaid);

    issue_pdf(&setup, invoice.id).await?;
    // A redelivered PDF event neither pays it nor emits again.
    issue_pdf(&setup, invoice.id).await?;
    let paid = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(paid.amount_due, 0);
    let paid_events = setup
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
        .filter(|event| matches!(event, OutboxEvent::InvoicePaid(e) if e.invoice_id == invoice.id))
        .count();
    assert_eq!(paid_events, 1, "invoice.paid fires exactly once");

    let credited = issue(
        &setup,
        note(invoice.id, CreditType::CreditToBalance, vec![]),
    )
    .await
    .map_err(test_error)?;
    assert_eq!(credited.credited_amount_cents, 3_000);
    assert_eq!(credited.debt_cancelled_amount_cents, 0);
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, grant).await,
        5_000,
        "the drawn credit returns to its grant"
    );
    assert_eq!(eur(&setup).await, 5_000, "and no cash is minted as credit");
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}
