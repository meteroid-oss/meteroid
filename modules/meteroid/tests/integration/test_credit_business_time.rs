//! Credit recorded at its business time: payments, invoice issues and expiries date the ledger and
//! the grants, not the moment they are processed.
use crate::data::ids::{CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID, USER_ID};
use crate::harness::{subscription, test_env_with_seed};
use crate::helpers::credit::{assert_ledger_balances, available, grant_remaining};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_customer_credit::{draft, line};
use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditGrantId, EventId, StoredDocumentId, WalletId};
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::domain::credit::GrantRollover;
use meteroid_store::domain::credit::{CreditGrantSource, GrantClassification, LedgerEntryKind};
use meteroid_store::domain::enums::InvoicePaymentStatus;
use meteroid_store::domain::outbox_event::InvoicePdfGeneratedEvent;
use meteroid_store::domain::price_components::CreditExpiry;
use meteroid_store::domain::{Invoice, LineItem, LineItemKind};
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::InvoiceInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::services::{InvoiceDelivery, IssueAt, SellCustomerCredit};
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

/// A whole-second moment `days` days back, so it survives the database's microsecond precision.
fn days_ago(days: i64) -> DateTime<Utc> {
    (Utc::now() - Duration::days(days))
        .date_naive()
        .and_hms_opt(9, 30, 0)
        .expect("valid time")
        .and_utc()
}

async fn wallet(setup: &MeteroidSetup) -> TestResult<WalletId> {
    let customer = setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?;
    Ok(setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &customer.currency)
        .await
        .map_err(test_error)?
        .id)
}

async fn fund_at(
    setup: &MeteroidSetup,
    amount_minor: i64,
    at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
) -> TestResult<CreditGrantId> {
    let wallet_id = wallet(setup).await?;
    Ok(setup
        .store
        .fund_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id,
                amount_minor,
                classification: GrantClassification::Paid,
                priority: DEFAULT_PRIORITY,
                expires_at,
                idempotency_key: Uuid::now_v7(),
                reason: "Business-time test credit".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            at,
        )
        .await
        .map_err(test_error)?
        .grant_id)
}

/// The wallet's statement, oldest first, as `(kind, amount, balance after, business time)`.
async fn statement(
    setup: &MeteroidSetup,
    wallet_id: WalletId,
) -> TestResult<Vec<(LedgerEntryKind, i64, i64, DateTime<Utc>)>> {
    let (entries, total) = setup
        .store
        .customer_credit_ledger(TENANT_ID, wallet_id, 100, 0)
        .await
        .map_err(test_error)?;
    assert_eq!(total, entries.len() as u64);
    Ok(entries
        .into_iter()
        .rev()
        .map(|e| {
            (
                e.kind,
                e.amount_minor,
                e.balance_after_minor,
                e.effective_at,
            )
        })
        .collect())
}

async fn orchestrate(setup: &MeteroidSetup) {
    run_once_invoice_orchestration(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
    )
    .await;
}

#[tokio::test]
async fn a_backdated_payment_funds_credit_from_when_it_was_paid_and_a_redelivery_funds_nothing()
-> TestResult {
    let setup = setup().await;
    let wallet_id = wallet(&setup).await?;
    let (sold_at, paid_at) = (days_ago(41), days_ago(40));
    let sold = setup
        .services
        .sell_customer_credit_at(
            Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id,
                amount_minor: 120_000,
                expires_in_months: Some(12),
                note: None,
                actor: Actor::User { id: USER_ID },
                unit_price: None,
                expected_unit_price: None,
            },
            sold_at,
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?;
    let invoice = sold.invoice.invoice;
    assert_eq!(invoice.invoice_date, sold_at.date_naive());
    assert_eq!(invoice.finalized_at, Some(sold_at.naive_utc()));

    let paid = setup
        .services
        .settle_invoice_payment_at(
            Actor::System,
            TENANT_ID,
            invoice.id,
            paid_at,
            Some("wire-1".to_owned()),
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(paid.paid_at, Some(paid_at.naive_utc()));

    let purchased = || async {
        setup
            .store
            .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
            .await
            .expect("wallets")
            .into_iter()
            .flat_map(|wallet| wallet.grants)
            .filter(|grant| grant.source == CreditGrantSource::Purchase)
            .collect::<Vec<_>>()
    };
    let grants = purchased().await;
    assert_eq!(grants.len(), 1, "the inline settlement funds the purchase");
    assert_eq!(grants[0].effective_at, paid_at);
    assert_eq!(
        grants[0].expires_at,
        paid_at.checked_add_months(Months::new(12)),
        "the term runs from the payment, not from processing"
    );
    let funded = available(&setup.store, TENANT_ID, CUST_UBER_ID, None).await;
    assert_eq!(funded, 120_000);

    // The consumer's delivery of the same settlement, and a redelivery of it, fund nothing more.
    orchestrate(&setup).await;
    let payment = setup
        .store
        .last_settled_payment_tx_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .expect("the recorded payment");
    setup
        .services
        .on_payment_transaction_settled(payment.into())
        .await
        .map_err(test_error)?;
    assert_eq!(purchased().await.len(), 1);
    assert_eq!(
        available(&setup.store, TENANT_ID, CUST_UBER_ID, None).await,
        funded
    );
    let funds = statement(&setup, wallet_id)
        .await?
        .into_iter()
        .filter(|(kind, ..)| *kind == LedgerEntryKind::Fund)
        .collect::<Vec<_>>();
    assert_eq!(
        funds,
        vec![(LedgerEntryKind::Fund, 120_000, 120_000, paid_at)]
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_backdated_invoice_draws_only_credit_in_effect_when_it_was_issued() -> TestResult {
    let setup = setup().await;
    let early = fund_at(&setup, 1_000, days_ago(30), None).await?;
    let late = fund_at(&setup, 5_000, days_ago(5), None).await?;
    let invoice = draft(&setup, vec![line("Seats", 3_000, false)]).await?;
    let issued_at = days_ago(10);
    let finalized = setup
        .services
        .finalize_invoice_at(
            Actor::System,
            invoice.id,
            TENANT_ID,
            IssueAt::At(issued_at),
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(finalized.finalized_at, Some(issued_at.naive_utc()));
    assert_eq!(finalized.applied_credits, 1_000);
    assert_eq!(finalized.amount_due, 2_000);
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, early).await,
        0
    );
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, CUST_UBER_ID, late).await,
        5_000,
        "credit granted after the issue is not drawn"
    );
    let draws = statement(&setup, wallet(&setup).await?)
        .await?
        .into_iter()
        .filter(|(kind, ..)| *kind == LedgerEntryKind::Draw)
        .collect::<Vec<_>>();
    assert_eq!(draws, vec![(LedgerEntryKind::Draw, -1_000, 0, issued_at)]);

    let ahead = draft(&setup, vec![line("Seats", 100, false)]).await?;
    assert!(
        setup
            .services
            .finalize_invoice_at(
                Actor::System,
                ahead.id,
                TENANT_ID,
                IssueAt::At(Utc::now() + Duration::hours(1)),
                InvoiceDelivery::Normal,
            )
            .await
            .is_err(),
        "an invoice is never issued in the future"
    );
    Ok(())
}

#[tokio::test]
async fn the_statement_follows_business_time_with_expiries_in_place() -> TestResult {
    let setup = setup().await;
    let wallet_id = wallet(&setup).await?;
    fund_at(&setup, 1_000, days_ago(30), Some(days_ago(20))).await?;
    fund_at(&setup, 2_000, days_ago(25), None).await?;
    let invoice = draft(&setup, vec![line("Seats", 500, false)]).await?;
    let applied = setup
        .services
        .finalize_invoice_at(
            Actor::System,
            invoice.id,
            TENANT_ID,
            IssueAt::At(days_ago(15)),
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?
        .invoice
        .applied_credits;
    assert_eq!(applied, 500, "the credit expired by then is not drawn");

    // Recorded after the draw, the expiry still sits at its date; unrecorded, it sits there too.
    let expected = vec![
        (LedgerEntryKind::Fund, 1_000, 1_000, days_ago(30)),
        (LedgerEntryKind::Fund, 2_000, 3_000, days_ago(25)),
        (LedgerEntryKind::Expiry, -1_000, 2_000, days_ago(20)),
        (LedgerEntryKind::Draw, -500, 1_500, days_ago(15)),
    ];
    assert_eq!(statement(&setup, wallet_id).await?, expected);
    let lapsed = setup
        .store
        .expire_customer_credit_for(TENANT_ID, CUST_UBER_ID, days_ago(1))
        .await
        .map_err(test_error)?;
    assert_eq!(lapsed, 1);
    assert_eq!(statement(&setup, wallet_id).await?, expected);
    assert_eq!(
        setup
            .store
            .expire_customer_credit_for(TENANT_ID, CUST_UBER_ID, days_ago(1))
            .await
            .map_err(test_error)?,
        0,
        "a lapse is recorded once"
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

#[tokio::test]
async fn a_zero_amount_invoice_is_paid_when_it_was_issued() -> TestResult {
    let setup = setup().await;
    let invoice = draft(&setup, vec![line("Seats", 0, false)]).await?;
    let issued_at = days_ago(3);
    setup
        .services
        .finalize_invoice_at(
            Actor::System,
            invoice.id,
            TENANT_ID,
            IssueAt::At(issued_at),
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?;
    setup
        .services
        .on_invoice_accounting_pdf_generated(
            InvoicePdfGeneratedEvent {
                id: EventId::new(),
                invoice_id: invoice.id,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                pdf_id: StoredDocumentId::new(),
                xml_id: None,
                einvoicing_status: None,
                einvoicing_report: None,
            },
            TENANT_ID,
        )
        .await
        .map_err(test_error)?;
    let paid = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);
    assert_eq!(paid.paid_at, Some(issued_at.naive_utc()));
    Ok(())
}

async fn sell_at(
    setup: &MeteroidSetup,
    amount_minor: i64,
    expires_in_months: Option<u32>,
    at: DateTime<Utc>,
) -> TestResult<Invoice> {
    let wallet_id = wallet(setup).await?;
    Ok(setup
        .services
        .sell_customer_credit_at(
            Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id,
                amount_minor,
                expires_in_months,
                note: None,
                actor: Actor::User { id: USER_ID },
                unit_price: None,
                expected_unit_price: None,
            },
            at,
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?
        .invoice
        .invoice)
}

async fn finalize_at(
    setup: &MeteroidSetup,
    invoice: common_domain::ids::InvoiceId,
    at: DateTime<Utc>,
) -> TestResult<Invoice> {
    Ok(setup
        .services
        .finalize_invoice_at(
            Actor::System,
            invoice,
            TENANT_ID,
            IssueAt::At(at),
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?
        .invoice)
}

async fn purchases(setup: &MeteroidSetup) -> Vec<meteroid_store::domain::credit::CreditGrant> {
    setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .expect("wallets")
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == CreditGrantSource::Purchase)
        .collect()
}

fn assert_never_negative(lines: &[(LedgerEntryKind, i64, i64, DateTime<Utc>)]) {
    assert!(
        lines.iter().all(|(_, _, balance, _)| *balance >= 0),
        "the running balance never goes negative: {lines:?}"
    );
}

/// A backdated draw never precedes credit given back after its date (old code dated invoice B's
/// draw Feb 1, before the void returned the value on Mar 5: +100, 0, -100, 0).
#[tokio::test]
async fn a_backdated_draw_of_value_given_back_later_keeps_the_balance_non_negative() -> TestResult {
    let setup = setup().await;
    let wallet_id = wallet(&setup).await?;
    fund_at(&setup, 100, days_ago(60), None).await?;
    let first = draft(&setup, vec![line("Seats", 100, false)]).await?;
    assert_eq!(
        finalize_at(&setup, first.id, days_ago(45))
            .await?
            .applied_credits,
        100
    );
    setup
        .store
        .void_invoice(Actor::System, first.id, TENANT_ID)
        .await
        .map_err(test_error)?;
    let second = draft(&setup, vec![line("Seats", 100, false)]).await?;
    assert_eq!(
        finalize_at(&setup, second.id, days_ago(30))
            .await?
            .applied_credits,
        100
    );

    let lines = statement(&setup, wallet_id).await?;
    assert_never_negative(&lines);
    let kinds = lines.iter().map(|(kind, ..)| *kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            LedgerEntryKind::Fund,
            LedgerEntryKind::Draw,
            LedgerEntryKind::DrawReversal,
            LedgerEntryKind::Draw,
        ]
    );
    assert_eq!(lines[3].3, lines[2].3, "dated when the value came back");
    assert_eq!(lines[3].2, 0);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

fn rolling_included_credit(amount: i64, start: NaiveDate, end: NaiveDate) -> LineItem {
    LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(amount),
        credit_grant_included: true,
        credit_grant_expiry: Some(CreditExpiry::BillingPeriod),
        credit_grant_rollover: Some(GrantRollover {
            max_minor: None,
            expiry: CreditExpiry::Month(1),
        }),
        start_date: start,
        end_date: end,
        ..line("Included credit", 0, false)
    }
}

/// Value given back after an expiry rolls over when it came back: the ROLLOVER and the new
/// grant's FUND share that date, while the new grant takes effect at the original expiry.
#[tokio::test]
async fn credit_rolled_over_after_a_late_give_back_is_dated_when_it_rolled() -> TestResult {
    let setup = setup().await;
    let wallet_id = wallet(&setup).await?;
    let (start, end) = (days_ago(20).date_naive(), days_ago(10).date_naive());
    let granting = draft(
        &setup,
        vec![
            rolling_included_credit(1_000, start, end),
            line("Seats", 2_000, false),
        ],
    )
    .await?;
    finalize_at(&setup, granting.id, days_ago(19)).await?;
    // Included credit pays its subscription's usage only.
    let spend = draft(&setup, vec![line("Usage", 400, true)]).await?;
    assert_eq!(
        finalize_at(&setup, spend.id, days_ago(15))
            .await?
            .promotional_credits,
        400,
        "included credit is promotional, taken off before tax"
    );
    // The void gives the 400 back to the expired grant, which then rolls over in full.
    setup
        .store
        .void_invoice(Actor::System, spend.id, TENANT_ID)
        .await
        .map_err(test_error)?;

    let lines = statement(&setup, wallet_id).await?;
    assert_never_negative(&lines);
    let kinds = lines.iter().map(|(kind, ..)| *kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            LedgerEntryKind::Fund,
            LedgerEntryKind::Draw,
            LedgerEntryKind::DrawReversal,
            LedgerEntryKind::Rollover,
            LedgerEntryKind::Fund,
        ]
    );
    let given_back_at = lines[2].3;
    assert!(given_back_at > end.and_hms_opt(0, 0, 0).expect("midnight").and_utc());
    assert_eq!(
        lines[3].3, given_back_at,
        "the rollover happens when the value is back"
    );
    assert_eq!(
        lines[4].3, given_back_at,
        "the rolled grant is funded then too"
    );
    assert_eq!(
        lines
            .iter()
            .map(|(_, _, balance, _)| *balance)
            .collect::<Vec<_>>(),
        vec![1_000, 600, 1_000, 0, 1_000]
    );
    let rolled = setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .find(|grant| grant.source == CreditGrantSource::Rollover)
        .expect("the rolled grant");
    let expired_at = end.and_hms_opt(0, 0, 0).expect("midnight").and_utc();
    assert_eq!(
        rolled.effective_at, expired_at,
        "spendable from the original expiry"
    );
    assert_eq!(rolled.amount_minor, 1_000);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// An operator's payment date funds a pack from that day; a payment received before the invoice
/// was issued is recorded as dated but funds from the issue, and a future date is refused.
#[tokio::test]
async fn a_manual_payment_date_funds_a_pack_from_its_payment_or_issue() -> TestResult {
    let setup = setup().await;
    let pay = |invoice: common_domain::ids::InvoiceId, date: DateTime<Utc>| {
        let services = setup.services.clone();
        async move {
            services
                .mark_invoice_as_paid(
                    Actor::System,
                    TENANT_ID,
                    invoice,
                    Decimal::new(50_000, 2),
                    date.fixed_offset(),
                    Some("wire".to_owned()),
                )
                .await
        }
    };
    let issued_at = days_ago(10);
    let prepaid = sell_at(&setup, 50_000, Some(12), issued_at).await?;
    assert!(
        pay(prepaid.id, Utc::now() + Duration::days(2))
            .await
            .is_err(),
        "a future payment date is refused"
    );
    assert!(purchases(&setup).await.is_empty());
    pay(prepaid.id, days_ago(12)).await.map_err(test_error)?;
    orchestrate(&setup).await;
    let paid = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, prepaid.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(
        paid.paid_at,
        Some(days_ago(12).naive_utc()),
        "recorded as dated"
    );
    let grants = purchases(&setup).await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].effective_at, issued_at, "funded from the issue");
    assert_eq!(
        grants[0].expires_at,
        issued_at.checked_add_months(Months::new(12))
    );

    let later = sell_at(&setup, 50_000, Some(12), issued_at).await?;
    let paid_at = days_ago(5);
    pay(later.id, paid_at).await.map_err(test_error)?;
    orchestrate(&setup).await;
    let grant = purchases(&setup)
        .await
        .into_iter()
        .find(|grant| grant.invoice_id == Some(later.id))
        .expect("the second pack");
    assert_eq!(grant.effective_at, paid_at);
    assert_eq!(
        grant.expires_at,
        paid_at.checked_add_months(Months::new(12))
    );
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// Scheduled events of a tenant on hold wait for the release, then fire as they would have.
#[tokio::test]
async fn scheduled_events_wait_for_the_lifecycle_hold_to_be_released() -> TestResult {
    use meteroid_store::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
    use meteroid_store::repositories::SubscriptionInterface;
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let start = (Utc::now().date_naive() - Months::new(2))
        .with_day(1)
        .expect("first of the month");
    // Not advanced automatically: the first period's invoice stays a draft for the event to finalize.
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let first = |invoices: Vec<Invoice>| {
        invoices
            .into_iter()
            .find(|invoice| invoice.billing_period_start == start)
            .expect("the first period's invoice")
    };
    let draft = first(env.get_invoices(sub_id).await);
    assert!(draft.can_edit());
    hold_lifecycle(&env).await?;
    let mut conn = env.conn().await;
    env.store()
        .schedule_events(
            &mut conn,
            vec![ScheduledEventNew {
                subscription_id: sub_id,
                tenant_id: TENANT_ID,
                scheduled_time: start.and_hms_opt(1, 0, 0).expect("time"),
                event_data: ScheduledEventData::FinalizeInvoice {
                    invoice_id: draft.id,
                },
                source: String::new(),
                created_by: Actor::System,
            }],
        )
        .await
        .map_err(test_error)?;
    drop(conn);

    env.process_due_events().await;
    assert!(
        first(env.get_invoices(sub_id).await).can_edit(),
        "a due event waits while the tenant is being seeded"
    );

    release_lifecycle(&env).await?;
    env.process_due_events().await;
    assert!(
        !first(env.get_invoices(sub_id).await).can_edit(),
        "it fires once the seed is finished"
    );
    Ok(())
}

/// Forgiving the rest of a partly paid purchase funds the paid share from when it was paid.
#[tokio::test]
async fn forgiveness_funds_the_paid_share_from_its_last_payment() -> TestResult {
    let setup = setup().await;
    let invoice = sell_at(&setup, 10_000, None, days_ago(10)).await?;
    let paid_at = days_ago(6);
    setup
        .services
        .add_manual_payment_transaction(
            Actor::System,
            TENANT_ID,
            invoice.id,
            Decimal::new(4_000, 2),
            paid_at.fixed_offset(),
            Some("partial".to_owned()),
        )
        .await
        .map_err(test_error)?;
    orchestrate(&setup).await;
    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: invoice.line_items[0].local_id.clone(),
                    amount_cents: 6_000,
                }],
                reason: Some("Forgiven".into()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    let settled = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(settled.payment_status, InvoicePaymentStatus::Paid);
    let grants = purchases(&setup).await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].amount_minor, 4_000, "only what was paid");
    assert_eq!(grants[0].effective_at, paid_at);
    assert_ledger_balances(&setup.store, TENANT_ID, CUST_UBER_ID).await;
    Ok(())
}

/// A replay holds the tenant's lifecycle, then bills a subscription several periods behind one
/// period at a time, each issued when its period ended and kept from collection.
#[tokio::test]
async fn a_replay_bills_one_period_at_a_time_at_its_end() -> TestResult {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let start = (Utc::now().date_naive() - Months::new(4))
        .with_day(1)
        .expect("first of the month");
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    hold_lifecycle(&env).await?;
    env.services()
        .get_and_process_cycle_transitions()
        .await
        .map_err(test_error)?;
    let held = env.get_subscription(sub_id).await;
    assert_eq!(
        held.current_period_start, start,
        "the worker leaves it alone"
    );

    let mut period_end = held.current_period_end.expect("a period end");
    for _ in 0..2 {
        let issued_at = period_end.and_hms_opt(9, 30, 0).expect("time").and_utc();
        assert!(
            env.services()
                .bill_subscription_period_at(
                    TENANT_ID,
                    sub_id,
                    issued_at - Duration::days(1),
                    InvoiceDelivery::Suppressed,
                )
                .await
                .is_err(),
            "the period has not ended a day earlier"
        );
        let invoice = env
            .services()
            .bill_subscription_period_at(TENANT_ID, sub_id, issued_at, InvoiceDelivery::Suppressed)
            .await
            .map_err(test_error)?
            .expect("the renewal bills")
            .invoice;
        assert_eq!(invoice.finalized_at, Some(issued_at.naive_utc()));
        assert_eq!(invoice.invoice_date, period_end);
        let mut conn = env.conn().await;
        assert!(
            diesel_models::replay_holds::is_invoice_delivery_held(&mut conn, TENANT_ID, invoice.id)
                .await
                .map_err(test_error)?,
            "a replayed invoice is neither collected nor sent"
        );
        let advanced = env.get_subscription(sub_id).await;
        assert_eq!(advanced.current_period_start, period_end);
        assert_eq!(advanced.error_count, 0);
        assert!(advanced.last_error.is_none());
        period_end = advanced.current_period_end.expect("a period end");
    }
    release_lifecycle(&env).await?;
    Ok(())
}

/// A sandbox seed in progress keeps the lifecycle worker off the tenant, as during a replay.
async fn hold_lifecycle(env: &crate::harness::TestEnv) -> TestResult {
    let mut conn = env.conn().await;
    diesel_models::sandbox_seed::SandboxSeedRow::start(&mut conn, TENANT_ID)
        .await
        .map_err(test_error)?;
    Ok(())
}

async fn release_lifecycle(env: &crate::harness::TestEnv) -> TestResult {
    let mut conn = env.conn().await;
    diesel_models::sandbox_seed::SandboxSeedRow::finish(&mut conn, TENANT_ID, None)
        .await
        .map_err(test_error)?;
    Ok(())
}
