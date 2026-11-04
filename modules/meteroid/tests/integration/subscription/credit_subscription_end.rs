//! A subscription's credit ends with it. An immediate cancellation credits the unused advance fees
//! and period packs exactly as an immediate plan change does; the subscription's grants then end at
//! the instant it ended, pay its usage up to there, and lapse. Customer-wide credit is untouched,
//! and a termination that missed setting the end (an older release) is healed by the sweep.
//! Other test binaries include this file: reach the harness through `meteroid_testkit`, not
//! `crate::harness`.
use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditGrantId, CustomerId, SubscriptionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::customer_credit::{CustomerCreditEntryRow, CustomerCreditGrantRow};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::coupons::{
    CouponCreditScope, CouponDiscount, CouponNew, SubscriptionRequired,
};
use meteroid_store::domain::credit::{
    CANCELLATION_REMOVAL, CreditApplication, CreditGrantSource, ENDED_WITH_SUBSCRIPTION,
    GrantClassification, GrantScope, PLAN_CHANGE_REMOVAL,
};
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::price_components::{
    CreditExpiry, PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::wallet_events::{CreditGrantExpiredEvent, GrantLapseReason};
use meteroid_store::domain::{Invoice, LineItemKind, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditNoteInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_testkit::harness::{TestEnv, subscription, test_env};

use super::credit_plan_change::{
    LARGE, SCALE, STUDIO, add_component, assert_revenue_sane, change_on, date, drawn_by,
    grant_rows, halyard, invoice, midnight, money, new_pack_product, pay, scale_usage, set_usage,
    start, the, the_note, usage_env,
};
use crate::data::ids::*;
use crate::helpers::credit::{Grant, assert_ledger_balances, fund_grant, grant_remaining};

/// A grant's movements as "KIND amount reason", in order; the reason only when one was recorded.
async fn entries(env: &TestEnv, grant: CreditGrantId) -> Vec<String> {
    let mut conn = env.conn().await;
    CustomerCreditEntryRow::list_by_grant(&mut conn, TENANT_ID, grant)
        .await
        .expect("movements")
        .into_iter()
        .map(|entry| match entry.reason {
            Some(reason) if entry.kind != "FUND" && entry.kind != "DRAW" => {
                format!("{} {} {reason}", entry.kind, entry.amount)
            }
            _ => format!("{} {}", entry.kind, entry.amount),
        })
        .collect()
}

async fn grant_row(
    env: &TestEnv,
    customer: CustomerId,
    id: CreditGrantId,
) -> CustomerCreditGrantRow {
    grant_rows(env, customer)
        .await
        .into_iter()
        .find(|grant| grant.id == id)
        .expect("the grant")
}

async fn cancel_on(env: &TestEnv, sub_id: SubscriptionId, on: CancellationEffectiveAt) {
    env.services()
        .cancel_subscription(Actor::System, sub_id, TENANT_ID, None, on)
        .await
        .expect("cancellation scheduled");
}

async fn sweep(env: &TestEnv, customer: CustomerId, at: chrono::DateTime<Utc>) {
    env.store()
        .expire_customer_credit_for(TENANT_ID, customer, at)
        .await
        .expect("expiry sweep");
}

async fn expired_events(env: &TestEnv, grant: CreditGrantId) -> Vec<CreditGrantExpiredEvent> {
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("webhook queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::CreditGrantExpired(expired) if expired.credit_grant_id == grant => {
                Some(*expired)
            }
            _ => None,
        })
        .collect()
}

fn terminal_invoice(invoices: &[Invoice], on: NaiveDate) -> &Invoice {
    invoices
        .iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .any(|line| line.metric_id.is_some() && line.end_date == on)
        })
        .unwrap_or_else(|| {
            let seen: Vec<_> = invoices
                .iter()
                .map(|i| {
                    (
                        i.invoice_date,
                        i.status,
                        i.line_items
                            .iter()
                            .map(|l| (l.name.clone(), l.amount_subtotal, l.start_date, l.end_date))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            panic!("no final invoice on {on}: {seen:?}")
        })
}

/// Large (Enterprise) includes EUR 100 of credit for the period, and bills bandwidth at EUR 0.10.
async fn included_credit_plan(env: &TestEnv, with_usage: bool) {
    add_component(
        env,
        "Included credit",
        LARGE,
        new_pack_product("Included credit"),
        Pricing::Credit {
            unit_price: Decimal::ZERO,
            quantity: 1,
            credit_amount: dec!(100),
            expiry: Some(CreditExpiry::BillingPeriod),
            credit_unit_id: None,
            rollover: None,
            once: false,
        },
    )
    .await;
    if with_usage {
        add_component(
            env,
            "Bandwidth",
            LARGE,
            ProductRef::New {
                name: "Bandwidth".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: METRIC_BANDWIDTH,
                    model: UsageModel::PerUnit,
                },
            },
            Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
        )
        .await;
    }
}

fn plan_credit<'a>(
    rows: &'a [CustomerCreditGrantRow],
    source_kind: &str,
) -> &'a CustomerCreditGrantRow {
    rows.iter()
        .find(|grant| grant.source_kind == source_kind && grant.subscription_id.is_some())
        .unwrap_or_else(|| panic!("no {source_kind} grant of a subscription"))
}

/// S7. Scale from Jan 1, paid; 400.00 of usage up to Jan 16; a EUR 100 top-up. Cancelled on
/// Jan 16 (16 of 31 days left): the pack's bonus then paid credit settle the usage (50 + 350), its
/// fee is credited back capped at the 150.00 of paid credit left (not the 258.06 the time is
/// worth), with the fees by the time left, on one note to balance. The welcome credit ends unused.
#[tokio::test]
async fn an_immediate_cancellation_credits_fees_and_pack_as_a_plan_change_does() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;
    let top_up = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant::invoice(10_000),
    )
    .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 4_000);

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 1, 16)),
    )
    .await;
    env.process_due_events().await;
    env.process_due_events().await;

    // 50000 × 16/31 = 25806 by time, capped at 15000 of paid credit left; 9900 × 16/31 = 5110,
    // 2500 × 16/31 = 1290.
    let ((kind, subtotal, tax, total), lines) = the_note(&env, &january).await;
    assert_eq!(kind, CreditType::CreditToBalance);
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -15_000, 0),
            ("Platform Fee".to_owned(), -5_110, 0),
            ("Seats".to_owned(), -1_290, 0),
        ]
    );
    assert_eq!((subtotal.abs(), tax, total.abs()), (21_400, 0, 21_400));

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    let welcome = the(&rows, "INCLUDED_CREDIT", "PROMOTIONAL", 5_000);
    assert_eq!(
        entries(&env, paid.id).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -35000".to_owned(),
            format!("REVERSAL -15000 {CANCELLATION_REMOVAL}"),
        ]
    );
    assert_eq!(
        entries(&env, bonus.id).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    for ended in [paid, bonus, welcome] {
        assert_eq!(ended.ends_at, Some(midnight(date(2024, 1, 16))));
        assert_eq!(ended.end_reason.as_deref(), Some("SUBSCRIPTION_ENDED"));
        assert_eq!(ended.subscription_id, Some(sub_id));
    }
    let note_credit = the(&rows, "CREDIT_NOTE", "PAID", 21_400);
    assert_eq!(note_credit.subscription_id, None);
    assert_eq!(note_credit.remaining, 21_400);

    let invoices = env.get_invoices(sub_id).await;
    let last = terminal_invoice(&invoices, date(2024, 1, 16));
    assert_eq!(
        money(last),
        (40_000, 5_000, 0, 35_000, 35_000, 0),
        "the final invoice takes the settlement over: bonus before tax, then paid credit"
    );

    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        entries(&env, welcome.id).await,
        vec![
            "FUND 5000".to_owned(),
            format!("EXPIRY -5000 {ENDED_WITH_SUBSCRIPTION}")
        ]
    );
    let expired = expired_events(&env, welcome.id).await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].reason, GrantLapseReason::SubscriptionEnded);
    assert_eq!(expired[0].subscription_id, Some(sub_id));
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, top_up).await,
        10_000,
        "customer-wide credit is untouched"
    );
    assert_eq!(grant_row(&env, CUST_UBER_ID, top_up).await.ends_at, None);
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(
            breakage, 0,
            "every unused paid cent was credited back; promotional credit lapses at cost 0"
        );
    }
}

/// S7, late event: as above, then 20.00 of usage dated before Jan 16 reaches the metering store
/// after the cancellation, within the settlement grace. The sweep settles it at the same bound
/// from the welcome credit ending then (a second settlement after the cancellation's), lapses the
/// other 30.00, and the final invoice takes both over: 420.00, 70.00 before tax, nothing due.
#[tokio::test]
async fn a_late_event_after_an_immediate_cancellation_is_paid_by_the_ending_credit() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;
    let top_up = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant::invoice(10_000),
    )
    .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 4_000);

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 1, 16)),
    )
    .await;
    env.process_due_events().await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let welcome = the(&rows, "INCLUDED_CREDIT", "PROMOTIONAL", 5_000).id;

    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 4_200);
    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        entries(&env, welcome).await,
        vec![
            "FUND 5000".to_owned(),
            "DRAW -2000".to_owned(),
            format!("EXPIRY -3000 {ENDED_WITH_SUBSCRIPTION}"),
        ]
    );
    let mut conn = env.conn().await;
    let settled: Vec<(i64, chrono::DateTime<Utc>)> =
        diesel_models::customer_credit::CreditUsageSettlementRow::list_for_period(
            &mut conn,
            TENANT_ID,
            sub_id,
            date(2024, 1, 1),
            "EUR",
        )
        .await
        .expect("settlements")
        .into_iter()
        .map(|settlement| (settlement.amount, settlement.event_time_bound))
        .collect();
    drop(conn);
    assert_eq!(
        settled,
        vec![
            (40_000, midnight(date(2024, 1, 16))),
            (2_000, midnight(date(2024, 1, 16)))
        ],
        "the cancellation's settlement, then the late usage's at the same bound"
    );

    env.process_due_events().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        money(terminal_invoice(&invoices, date(2024, 1, 16))),
        (42_000, 7_000, 0, 35_000, 35_000, 0),
        "bonus 50.00 and welcome 20.00 before tax, paid credit 350.00"
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, top_up).await,
        10_000
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(breakage, 0);
    }
}

/// Studio cancelled on Feb 15 while February's invoice (charged in advance, no auto-advance) is
/// still a draft: nothing is credited then. Issued later, it gets the note the cancellation would
/// have issued: 2900 and 1000 × 15/29 cancelled, its debt reduced; issuing or sweeping again
/// credits nothing more.
#[tokio::test]
async fn a_cancellation_while_the_period_invoice_is_a_draft_credits_it_once_issued() {
    let (env, _usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(STUDIO)
        .start_date(date(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    assert_eq!(
        february.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft
    );

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 2, 15)),
    )
    .await;
    env.process_due_events().await;
    assert!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, february.id)
            .await
            .expect("notes")
            .is_empty(),
        "a draft is never credited nor changed"
    );

    env.manual_finalize_invoice(february.id)
        .await
        .expect("finalized");
    let ((kind, subtotal, tax, total), lines) = the_note(&env, &february).await;
    assert_eq!(
        (kind, subtotal, tax, total),
        (CreditType::DebtCancellation, -2_017, 0, -2_017)
    );
    assert_eq!(
        lines,
        vec![
            ("Platform Fee".to_owned(), -1_500, 0),
            ("Seats".to_owned(), -517, 0)
        ]
    );
    let issued = env.get_detailed_invoice(february.id).await.invoice;
    assert_eq!(issued.amount_due, 3_900 - 2_017);

    assert_eq!(
        env.services()
            .settle_missed_plan_change_endings(100)
            .await
            .expect("sweep"),
        0
    );
    assert_eq!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, february.id)
            .await
            .expect("notes")
            .len(),
        1
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Studio cancelled on Feb 15 while February's invoice is a draft; a card charge captured later
/// issues it, so the cancellation's note keeps the money as balance credit dated Feb 15.
#[tokio::test]
async fn a_note_on_an_invoice_issued_after_its_cancellation_is_dated_from_the_invoice() {
    use diesel_models::enums::{PaymentStatusEnum, PaymentTypeEnum};
    use diesel_models::payments::PaymentTransactionRowNew;
    use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

    let (env, _usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(STUDIO)
        .start_date(date(2024, 1, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    assert_eq!(
        february.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft
    );

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 2, 15)),
    )
    .await;
    env.process_due_events().await;

    // A charge of the draft (a checkout acceptance) captured now: its settlement issues it.
    PaymentTransactionRowNew {
        id: common_domain::ids::PaymentTransactionId::new(),
        tenant_id: TENANT_ID,
        invoice_id: Some(february.id),
        provider_transaction_id: Some("ch_after_cancellation".to_string()),
        amount: february.total,
        currency: february.currency.clone(),
        payment_method_id: None,
        status: PaymentStatusEnum::Settled,
        payment_type: PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: Some(Utc::now().naive_utc()),
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
    .insert(&mut env.conn().await)
    .await
    .expect("the captured charge");
    let captured = env
        .store()
        .last_settled_payment_tx_by_invoice_id(TENANT_ID, february.id)
        .await
        .expect("the charge")
        .expect("settled");
    env.services()
        .on_payment_transaction_settled(captured.into())
        .await
        .expect("the settlement issues the draft");
    let issued = env.get_detailed_invoice(february.id).await.invoice;
    assert!(issued.finalized_at.is_some(), "the settlement issued it");
    let ((kind, _, _, _), _) = the_note(&env, &february).await;
    assert_eq!(kind, CreditType::CreditToBalance, "the money was collected");
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Scale from Jan 1 with `units` of January bandwidth (0.10 each); January's invoice is paid only
/// after February's invoice billed that usage. Returns January's and February's invoices.
async fn pack_paid_after_its_period(units: i64) -> (TestEnv, Invoice, Invoice) {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 2, 1), units);
    env.process_cycles().await;
    let february = invoice(&env, sub_id, 1).await;
    assert_eq!(
        february.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Finalized
    );
    pay(&env, &january).await;
    assert_eq!(
        money(&invoice(&env, sub_id, 1).await),
        money(&february),
        "February stays as invoiced"
    );
    (env, january, february)
}

/// January's pack (500.00 paid + 50.00 bonus) movements, by classification.
async fn january_pack(env: &TestEnv, january: &Invoice, classification: &str) -> Vec<String> {
    let rows = grant_rows(env, CUST_UBER_ID).await;
    let grant = rows
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == classification
                && g.invoice_id == Some(january.id)
        })
        .unwrap_or_else(|| panic!("January's {classification} pack credit"));
    assert_eq!(grant.remaining, 0, "nothing of it is left to spend");
    entries(env, grant.id).await
}

/// The money wallet's dashboard totals (added, removed, expired) and its added groups' sources.
async fn wallet_activity(env: &TestEnv) -> ((i64, i64, i64), Vec<CreditGrantSource>) {
    use meteroid_store::repositories::credit::CustomerCreditActivityInterface;
    let activity = env
        .store()
        .customer_credit_activity(TENANT_ID, CUST_UBER_ID, None, None)
        .await
        .expect("credit activity");
    assert_eq!(activity.len(), 1, "one wallet");
    let wallet = &activity[0];
    (
        (
            wallet.credit_added_minor,
            wallet.removed_minor,
            wallet.expired_minor,
        ),
        wallet
            .credit_added_groups
            .iter()
            .map(|group| group.source)
            .collect(),
    )
}

/// S9-late (OQ11): with no January usage, an on-time pack would have lapsed whole: so does the
/// late one, at once. No note; the 500.00 paid is breakage.
#[tokio::test]
async fn a_pack_paid_after_an_unused_period_lapses_whole() {
    use meteroid_store::domain::credit::LATE_PAYMENT_REMOVAL;
    let (env, january, _) = pack_paid_after_its_period(0).await;

    assert!(
        notes_of(&env, &january).await.is_empty(),
        "nothing to credit back"
    );
    assert_eq!(
        january_pack(&env, &january, "PAID").await,
        vec![
            "FUND 50000".to_owned(),
            format!("EXPIRY -50000 {LATE_PAYMENT_REMOVAL}"),
        ]
    );
    assert_eq!(
        january_pack(&env, &january, "PROMOTIONAL").await,
        vec![
            "FUND 5000".to_owned(),
            format!("EXPIRY -5000 {LATE_PAYMENT_REMOVAL}"),
        ]
    );
    assert!(
        grant_rows(&env, CUST_UBER_ID)
            .await
            .iter()
            .all(|g| g.source_kind != "CREDIT_NOTE"),
        "no balance credit"
    );
    assert_eq!(
        wallet_activity(&env).await.0,
        (50_000 + 5_000 + 5_000, 0, 55_000),
        "the pack, its bonus and the welcome credit added; the pack expired whole"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(breakage, 50_000);
    }
}

/// S9-late (OQ11): January's 400.00 of usage, billed on February's invoice with the welcome
/// credit's 50.00 taken off it, is what the pack would have paid on time: its bonus 50.00 first,
/// then 350.00 of its paid credit. All 400.00 comes back on a note to the balance, reversing as
/// much paid credit, so late pays what on time does; the other 100.00 and the bonus lapse.
#[tokio::test]
async fn a_pack_paid_after_its_period_was_invoiced_credits_back_what_it_would_have_paid() {
    use meteroid_store::domain::credit::LATE_PAYMENT_REMOVAL;
    let (env, january, february) = pack_paid_after_its_period(4_000).await;
    assert_eq!(
        money(&february),
        (102_400, 5_000, 0, 97_400, 0, 97_400),
        "January's usage less the welcome credit, February's pack and fees; no pack credit yet"
    );

    let ((kind, subtotal, tax, total), lines) = the_note(&env, &january).await;
    assert_eq!(
        (kind, subtotal, tax, total),
        (CreditType::CreditToBalance, -40_000, 0, -40_000)
    );
    assert_eq!(lines, vec![("Monthly credit".to_owned(), -40_000, 0)]);
    assert_eq!(
        january_pack(&env, &january, "PAID").await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -40000 {LATE_PAYMENT_REMOVAL}"),
            format!("EXPIRY -10000 {LATE_PAYMENT_REMOVAL}"),
        ]
    );
    assert_eq!(
        january_pack(&env, &january, "PROMOTIONAL").await,
        vec![
            "FUND 5000".to_owned(),
            format!("EXPIRY -5000 {LATE_PAYMENT_REMOVAL}"),
        ]
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let balance = the(&rows, "CREDIT_NOTE", "PAID", 40_000);
    assert_eq!(
        (
            balance.remaining,
            balance.subscription_id,
            balance.expires_at
        ),
        (40_000, None, None)
    );
    let ((added, removed, expired), sources) = wallet_activity(&env).await;
    assert_eq!(
        (added, removed, expired),
        (50_000 + 5_000 + 5_000, 0, 10_000 + 5_000),
        "the pack, its bonus and the welcome credit added; the note's balance counted once"
    );
    assert!(
        !sources.contains(&CreditGrantSource::CreditNote),
        "{sources:?}"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(
            breakage, 10_000,
            "the fee beyond what January would have drawn, bonus included, is breakage"
        );
    }
}

/// S9-late (OQ11): January's 700.00 of usage would have drawn the whole pack: its whole fee is
/// credited back, its paid credit reversed with it (no breakage), its bonus lapsing at no cost.
#[tokio::test]
async fn a_pack_paid_after_a_period_it_would_have_used_up_is_credited_back_whole() {
    use meteroid_store::domain::credit::LATE_PAYMENT_REMOVAL;
    let (env, january, _) = pack_paid_after_its_period(7_000).await;

    let ((kind, subtotal, tax, total), lines) = the_note(&env, &january).await;
    assert_eq!(
        (kind, subtotal, tax, total),
        (CreditType::CreditToBalance, -50_000, 0, -50_000)
    );
    assert_eq!(lines, vec![("Monthly credit".to_owned(), -50_000, 0)]);
    assert_eq!(
        january_pack(&env, &january, "PAID").await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -50000 {LATE_PAYMENT_REMOVAL}"),
        ]
    );
    assert_eq!(
        january_pack(&env, &january, "PROMOTIONAL").await,
        vec![
            "FUND 5000".to_owned(),
            format!("EXPIRY -5000 {LATE_PAYMENT_REMOVAL}"),
        ]
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    assert_eq!(the(&rows, "CREDIT_NOTE", "PAID", 50_000).remaining, 50_000);
    let ((added, removed, expired), sources) = wallet_activity(&env).await;
    assert_eq!(
        (added, removed, expired),
        (50_000 + 5_000 + 5_000, 0, 5_000),
        "the pack, its bonus and the welcome credit added; the note's balance counted once"
    );
    assert!(
        !sources.contains(&CreditGrantSource::CreditNote),
        "{sources:?}"
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(
            breakage, 0,
            "the pack's money is credited back, not forfeited"
        );
    }
}

/// Pays an invoice by hand on `paid_on`, then settles the payment.
async fn pay_on(env: &TestEnv, invoice: &Invoice, paid_on: chrono::NaiveDateTime) {
    env.services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice.id,
            Decimal::new(invoice.amount_due, 2),
            paid_on.and_utc().fixed_offset(),
            Some(format!("payment-{}", uuid::Uuid::now_v7())),
        )
        .await
        .expect("the invoice is paid");
    meteroid::workers::pgmq::processors::run_once_invoice_orchestration(
        std::sync::Arc::new(env.store().clone()),
        std::sync::Arc::new(env.services().clone()),
    )
    .await;
}

/// Reverses the invoice's payment as a failed collection (the invoice reopens), then pays it
/// again today.
async fn charge_back_and_pay_again(env: &TestEnv, invoice: &Invoice) {
    use meteroid_store::domain::enums::ReversalKind;
    use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
    use meteroid_store::services::ReverseManualPaymentCommand;
    let payment = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("payments")
        .into_iter()
        .map(|payment| payment.transaction)
        .find(|payment| payment.amount > 0)
        .expect("the payment");
    env.services()
        .reverse_manual_payment(
            Actor::System,
            TENANT_ID,
            ReverseManualPaymentCommand {
                transaction_id: payment.id,
                kind: ReversalKind::Returned,
                reason: Some("the wire bounced".to_owned()),
                reversed_at: Utc::now().naive_utc(),
            },
        )
        .await
        .expect("charged back");
    let reopened = env.get_detailed_invoice(invoice.id).await.invoice;
    pay_on(env, &reopened, Utc::now().naive_utc()).await;
}

/// Scale started 40 days ago, its first invoice issued and paid on time: the pack was funded
/// during its period and drawn by its usage. Charged back and paid again after the period, the
/// pack is not "paid late": no note, nothing credited back.
#[tokio::test]
async fn a_pack_paid_on_time_then_charged_back_and_paid_again_is_not_credited_back() {
    use diesel_async::SimpleAsyncConnection;
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let start_on = Utc::now().date_naive() - Duration::days(40);
    let sub_id = start(&env, CUST_UBER_ID, SCALE, start_on).await;
    let period_end = env
        .get_subscription(sub_id)
        .await
        .current_period_end
        .expect("period end");
    let first = invoice(&env, sub_id, 0).await;
    env.conn()
        .await
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE invoice SET finalized_at = '{start_on} 00:00:00' WHERE id = '{}';
             SET session_replication_role = origin;",
            first.id.as_uuid()
        ))
        .await
        .expect("issued when its period opened");
    pay_on(
        &env,
        &first,
        (start_on + Duration::days(1)).and_time(NaiveTime::MIN),
    )
    .await;
    set_usage(&usage, start_on, period_end, 3_000);
    env.process_cycles().await;
    assert_eq!(env.get_invoices(sub_id).await.len(), 2);

    charge_back_and_pay_again(&env, &first).await;

    assert!(
        notes_of(&env, &first).await.is_empty(),
        "funded during its period: never credited back"
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000).id;
    assert!(
        entries(&env, paid)
            .await
            .iter()
            .all(|entry| !entry.contains("paid after its period")),
        "{:?}",
        entries(&env, paid).await
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// Scale's first invoice paid after its period ended while the next invoice was still a draft:
/// the pack is not credited back, it pays its period's usage on that invoice (bonus 50.00, paid
/// 50.00). Charged back and paid again once that invoice is issued, there is nothing more it would
/// have paid on time: no note, the 450.00 left lapses as it would have at the period end.
#[tokio::test]
async fn a_late_pack_drawn_by_its_period_invoice_is_not_credited_back_for_what_it_could_not_use() {
    use meteroid_store::domain::credit::LATE_PAYMENT_REMOVAL;
    let (env, usage) = usage_env().await;
    env.set_consolidate_and_grace(false, 720).await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let start_on = Utc::now().date_naive() - Duration::days(40);
    let sub_id = start(&env, CUST_UBER_ID, SCALE, start_on).await;
    let period_end = env
        .get_subscription(sub_id)
        .await
        .current_period_end
        .expect("period end");
    let first = invoice(&env, sub_id, 0).await;
    set_usage(&usage, start_on, period_end, 1_000);
    env.process_cycles().await;
    let next = invoice(&env, sub_id, 1).await;
    assert_eq!(
        next.status,
        meteroid_store::domain::enums::InvoiceStatusEnum::Draft
    );

    pay(&env, &first).await;
    assert!(
        notes_of(&env, &first).await.is_empty(),
        "its period is still billable on the draft"
    );
    env.manual_finalize_invoice(next.id)
        .await
        .expect("finalized");
    let next = env.get_detailed_invoice(next.id).await.invoice;
    assert_eq!(
        (next.promotional_credits, next.applied_credits),
        (5_000, 5_000),
        "the late pack pays its period's usage"
    );
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000).id;

    charge_back_and_pay_again(&env, &first).await;

    assert!(notes_of(&env, &first).await.is_empty(), "it paid its usage");
    assert_eq!(
        entries(&env, paid).await.last().cloned(),
        Some(format!("EXPIRY -45000 {LATE_PAYMENT_REMOVAL}"))
    );
    assert_eq!(grant_row(&env, CUST_UBER_ID, paid).await.remaining, 0);
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// OQ1 parity with `a_forced_downgrade_refunds_the_unused_time_...`: cancelled on Jan 16 with no
/// usage, the same 258.06 + 51.10 + 12.90 come back on the same note; the 241.94 of paid credit
/// not refunded ends with the subscription, the same breakage as the plan change forfeits.
#[rstest]
#[tokio::test]
async fn a_cancellation_refunds_the_unused_time_exactly_like_a_forced_downgrade(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let january = invoice(&env, sub_id, 0).await;
    pay(&env, &january).await;

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 1, 16)),
    )
    .await;
    env.process_due_events().await;
    env.process_due_events().await;

    let (_, lines) = the_note(&env, &january).await;
    assert_eq!(
        lines,
        vec![
            ("Monthly credit".to_owned(), -25_806, 0),
            ("Platform Fee".to_owned(), -5_110, 0),
            ("Seats".to_owned(), -1_290, 0),
        ]
    );
    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    assert_eq!(
        entries(&env, the(&rows, "CREDIT_PURCHASE", "PAID", 50_000).id).await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -25806 {CANCELLATION_REMOVAL}"),
            format!("EXPIRY -24194 {ENDED_WITH_SUBSCRIPTION}"),
        ]
    );
    assert_eq!(
        entries(&env, the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000).id).await,
        vec![
            "FUND 5000".to_owned(),
            format!("EXPIRY -5000 {ENDED_WITH_SUBSCRIPTION}")
        ]
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(breakage, 24_194);
    }
}

/// T3. EUR 100 included for March, a EUR 50 top-up; 65.00 of usage known when the subscription is
/// cancelled on Mar 20, 5.00 more dated before it arriving after. The included credit ends then;
/// the sweep settles all 70.00 from it and lapses 30.00 "Ended with subscription"; the final
/// invoice takes the 70.00 over and nothing is due. The top-up is untouched.
#[tokio::test]
async fn included_credit_ends_with_its_subscription_and_pays_its_late_usage() {
    let (env, usage) = usage_env().await;
    included_credit_plan(&env, true).await;
    let sub_id = start(&env, CUST_UBER_ID, LARGE, date(2024, 3, 1)).await;
    let top_up = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant::invoice(5_000),
    )
    .await;
    set_usage(&usage, date(2024, 3, 1), date(2024, 3, 20), 650);

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 3, 20)),
    )
    .await;
    env.process_due_events().await;

    let pack = plan_credit(&grant_rows(&env, CUST_UBER_ID).await, "INCLUDED_CREDIT").clone();
    assert_eq!(pack.amount, 10_000);
    assert_eq!(pack.application, CreditApplication::Usage.as_db_str());
    assert_eq!(pack.ends_at, Some(midnight(date(2024, 3, 20))));
    assert_eq!(pack.end_reason.as_deref(), Some("SUBSCRIPTION_ENDED"));
    assert_eq!(
        entries(&env, pack.id).await,
        vec!["FUND 10000"],
        "credit that refunds nothing is settled once, by the sweep after the grace"
    );

    set_usage(&usage, date(2024, 3, 1), date(2024, 3, 20), 700);
    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 3, 20)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        entries(&env, pack.id).await,
        vec![
            "FUND 10000".to_owned(),
            "DRAW -7000".to_owned(),
            format!("EXPIRY -3000 {ENDED_WITH_SUBSCRIPTION}"),
        ]
    );
    let expired = expired_events(&env, pack.id).await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].expired_minor, 3_000);
    assert_eq!(expired[0].reason, GrantLapseReason::SubscriptionEnded);
    assert_eq!(expired[0].subscription_id, Some(sub_id));

    env.process_due_events().await;
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        money(terminal_invoice(&invoices, date(2024, 3, 20))),
        (7_000, 7_000, 0, 0, 0, 0)
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, top_up).await,
        5_000
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(breakage, 0);
    }
}

/// S6. Scale cancelled at the end of January with 400.00 of usage and a EUR 100 top-up: every
/// plan grant ends on Feb 1, which is also the pack's expiry, so they lapse "Ended with
/// subscription" (OQ18). Lapsing at that bound they pay first: bonus and welcome before tax
/// (100.00), paid credit 300.00. The 200.00 of paid credit left is breakage; the top-up untouched.
#[tokio::test]
async fn a_cancellation_at_period_end_ends_the_plan_credit_with_its_expiry() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let sub_id = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    let top_up = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant::invoice(10_000),
    )
    .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 2, 1), 4_000);

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 2, 1)),
    )
    .await;
    env.process_cycles().await;
    env.process_due_events().await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 2, "no February pack is billed");
    assert_eq!(money(&invoices[1]), (40_000, 10_000, 0, 30_000, 30_000, 0));
    assert!(
        env.store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoices[0].id)
            .await
            .expect("notes")
            .is_empty(),
        "nothing unused to credit at the period's end"
    );

    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 1)) + Duration::hours(1),
    )
    .await;
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    assert_eq!(paid.ends_at, paid.expires_at);
    assert_eq!(
        entries(&env, paid.id).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -30000".to_owned(),
            format!("EXPIRY -20000 {ENDED_WITH_SUBSCRIPTION}"),
        ]
    );
    let expired = expired_events(&env, paid.id).await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].reason, GrantLapseReason::SubscriptionEnded);
    assert_eq!(expired[0].subscription_id, Some(sub_id));
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, top_up).await,
        10_000
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
    if let Some(breakage) = assert_revenue_sane(&env, CUST_UBER_ID).await {
        assert_eq!(breakage, 20_000);
    }
}

/// Mixed-version deploy: an older release cancels the subscription without ending its credit. The
/// sweep's first step ends it at the instant the subscription ended, once, and the credit lapses
/// "Ended with subscription"; customer-wide credit is untouched.
#[rstest]
#[tokio::test]
async fn the_sweep_ends_the_credit_a_termination_left_running(#[future] test_env: TestEnv) {
    let env = test_env.await;
    included_credit_plan(&env, false).await;
    let sub_id = start(&env, CUST_UBER_ID, LARGE, date(2024, 3, 1)).await;
    let top_up = fund_grant(
        env.store(),
        TENANT_ID,
        CUST_UBER_ID,
        Some("EUR"),
        Grant::invoice(5_000),
    )
    .await;
    {
        use diesel_async::SimpleAsyncConnection;
        env.conn()
            .await
            .batch_execute(&format!(
                "UPDATE subscription SET status = 'CANCELLED', current_period_start = '2024-03-20',
                     current_period_end = NULL, next_cycle_action = NULL
                 WHERE id = '{}'",
                sub_id.as_uuid()
            ))
            .await
            .expect("cancelled by an older release");
    }
    let pack = plan_credit(&grant_rows(&env, CUST_UBER_ID).await, "INCLUDED_CREDIT").id;
    assert_eq!(grant_row(&env, CUST_UBER_ID, pack).await.ends_at, None);

    assert_eq!(
        env.store()
            .end_credit_of_ended_subscriptions(100)
            .await
            .expect("repair"),
        1
    );
    assert_eq!(
        env.store()
            .end_credit_of_ended_subscriptions(100)
            .await
            .expect("repair again"),
        0,
        "repaired once"
    );
    assert_eq!(
        grant_row(&env, CUST_UBER_ID, pack).await.ends_at,
        Some(midnight(date(2024, 3, 20)))
    );
    assert_eq!(grant_row(&env, CUST_UBER_ID, top_up).await.ends_at, None);

    env.store()
        .expire_customer_credit(100)
        .await
        .expect("the sweep");
    assert_eq!(
        entries(&env, pack).await,
        vec![
            "FUND 10000".to_owned(),
            format!("EXPIRY -10000 {ENDED_WITH_SUBSCRIPTION}")
        ]
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, top_up).await,
        5_000
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

fn set_database_usage(usage: &MutableMockUsageClient, from: NaiveDate, to: NaiveDate, units: i64) {
    let (start, end) = (from.and_time(NaiveTime::MIN), to.and_time(NaiveTime::MIN));
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_DATABASE_SIZE,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(units),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
}

/// T4. A on Scale with 150.00 of usage, B on Large with 10.00 of its own; A forced to Studio on
/// Jan 16. A's pack settles A's usage only (bonus 50, paid 100), its fee comes back capped by time
/// (258.06 of 400.00 left), the rest is removed; the welcome credit stays A's, unchanged; Studio's
/// credits are A's too, and the adjustment never draws the removed pack.
#[tokio::test]
async fn a_plan_change_keeps_the_subscription_s_scoped_credit() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    add_component(
        &env,
        "DB Storage",
        LARGE,
        ProductRef::New {
            name: "DB Storage".to_owned(),
            fee_type: FeeTypeEnum::Usage,
            fee_structure: FeeStructure::Usage {
                metric_id: METRIC_DATABASE_SIZE,
                model: UsageModel::PerUnit,
            },
        },
        Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(0.10) }),
    )
    .await;
    let a = start(&env, CUST_UBER_ID, SCALE, date(2024, 1, 1)).await;
    let b = start(&env, CUST_UBER_ID, LARGE, date(2024, 1, 1)).await;
    pay(&env, &invoice(&env, a, 0).await).await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 1_500);
    set_database_usage(&usage, date(2024, 1, 1), date(2024, 1, 16), 100);
    let before = grant_rows(&env, CUST_UBER_ID).await;
    let welcome = the(&before, "INCLUDED_CREDIT", "PROMOTIONAL", 5_000).clone();

    change_on(&env, a, STUDIO, date(2024, 1, 16)).await;
    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 1, 16)) + Duration::hours(1),
    )
    .await;

    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = the(&rows, "CREDIT_PURCHASE", "PAID", 50_000);
    let bonus = the(&rows, "CREDIT_PURCHASE", "PROMOTIONAL", 5_000);
    assert_eq!(
        entries(&env, paid.id).await,
        vec![
            "FUND 50000".to_owned(),
            "DRAW -10000".to_owned(),
            format!("REVERSAL -25806 {PLAN_CHANGE_REMOVAL}"),
            format!("EXPIRY -14194 {PLAN_CHANGE_REMOVAL}"),
        ],
        "B's usage never draws A's pack"
    );
    assert_eq!(
        entries(&env, bonus.id).await,
        vec!["FUND 5000", "DRAW -5000"]
    );
    let kept = the(&rows, "INCLUDED_CREDIT", "PROMOTIONAL", 5_000);
    assert_eq!(
        (kept.id, kept.remaining, kept.subscription_id, kept.ends_at),
        (welcome.id, 5_000, Some(a), None)
    );
    let studio = rows
        .iter()
        .find(|g| g.source_kind == "INCLUDED_CREDIT" && g.amount == 258_065)
        .expect("Studio's credits: 500000 × 16/31");
    assert_eq!(studio.subscription_id, Some(a));
    let adjustment = invoice(&env, a, 1).await;
    let drew = drawn_by(&env, &adjustment).await;
    assert!(!drew.contains(&paid.id) && !drew.contains(&bonus.id));
    assert!(
        rows.iter()
            .filter(|g| g.subscription_id.is_some())
            .all(|g| g.subscription_id == Some(a)),
        "B has no plan credit"
    );
    let _ = b;
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

fn credit_coupon(code: &str, scope: CouponCreditScope) -> CouponNew {
    CouponNew {
        code: code.to_owned(),
        description: "Launch offer".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Credit {
            currency: "EUR".to_owned(),
            amount: dec!(30),
            expires_in_days: Some(25),
            scope,
        },
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: false,
        plan_ids: vec![],
    }
}

#[derive(diesel::QueryableByName)]
struct AppliedOn {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    subscription_id: uuid::Uuid,
}

/// T6 / S12. A promo code for one subscription, redeemed by a customer with two: refused without
/// one named (the refusal lists both, oldest first); named, the credit is that subscription's, and
/// ends with it. A customer-wide code needs no choice and lands on the oldest subscription.
#[rstest]
#[tokio::test]
async fn a_subscription_promo_code_needs_its_subscription_named(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    let started = |customer| {
        subscription()
            .customer(customer)
            .plan_version(PLAN_VERSION_STARTER_ID)
            .start_date(Utc::now().date_naive())
            .on_start()
            .no_trial()
            .create(env.services())
    };
    let a = started(CUST_UBER_ID).await;
    let b = started(CUST_UBER_ID).await;
    let elsewhere = started(CUST_SPOTIFY_ID).await;
    env.create_coupon(credit_coupon("LAUNCHSUB", CouponCreditScope::Subscription))
        .await;
    let launch = env
        .create_coupon(credit_coupon("LAUNCH100", CouponCreditScope::Customer))
        .await;

    let refused = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "launchsub", None)
        .await
        .expect_err("a subscription is required");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(message)
            if message == "Choose the subscription this code applies to."
    ));
    let SubscriptionRequired(choices) = refused
        .frames()
        .find_map(|frame| frame.downcast_ref::<SubscriptionRequired>())
        .cloned()
        .expect("the subscriptions to choose from");
    assert_eq!(choices.iter().map(|c| c.id).collect::<Vec<_>>(), vec![a, b]);
    let not_ours = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "LAUNCHSUB", Some(elsewhere))
        .await
        .expect_err("another customer's subscription");
    assert!(format!("{not_ours:?}").contains("This code can't be redeemed on your account."));

    let redeemed = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "LAUNCHSUB", Some(b))
        .await
        .expect("redeemed on B");
    assert_eq!(redeemed.amount_minor, 3_000);
    env.services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "LAUNCH100", None)
        .await
        .expect("a customer-wide code needs no choice");
    let mut conn = env.conn().await;
    let applied: AppliedOn =
        diesel::sql_query("SELECT subscription_id FROM applied_coupon WHERE coupon_id = $1")
            .bind::<diesel::sql_types::Uuid, _>(launch.as_uuid())
            .get_result(&mut conn)
            .await
            .expect("the applied coupon");
    assert_eq!(
        applied.subscription_id,
        a.as_uuid(),
        "the oldest subscription"
    );
    drop(conn);

    let coupon_grants = |env: &TestEnv| {
        let store = env.store().clone();
        async move {
            store
                .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
                .await
                .expect("wallets")
                .into_iter()
                .flat_map(|wallet| wallet.grants)
                .filter(|grant| grant.source == CreditGrantSource::Coupon)
                .collect::<Vec<_>>()
        }
    };
    let granted = coupon_grants(&env).await;
    let for_b = granted
        .iter()
        .find(|g| g.reference.as_deref() == Some("LAUNCHSUB"))
        .expect("B's promo credit");
    assert_eq!(for_b.scope.subscription_id, Some(b));
    assert_eq!(for_b.classification, GrantClassification::Promotional);
    let shared = granted
        .iter()
        .find(|g| g.reference.as_deref() == Some("LAUNCH100"))
        .expect("the customer-wide promo credit");
    assert_eq!(shared.scope, GrantScope::customer_wide());

    cancel_on(&env, b, CancellationEffectiveAt::Immediate).await;
    env.process_due_events().await;
    let granted = coupon_grants(&env).await;
    let ended = granted.iter().find(|g| g.id == for_b.id).unwrap();
    assert!(ended.ends_at.is_some(), "B's promo credit ends with B");
    let untouched = granted.iter().find(|g| g.id == shared.id).unwrap();
    assert_eq!(untouched.ends_at, None);
}

/// T11. A credit note gives credit back to the scoped grant that paid the line; after the
/// subscription ended, what goes back waits out the settlement grace and lapses with the rest at
/// the sweep; credit beyond the draws is customer-wide.
#[rstest]
#[tokio::test]
async fn credit_given_back_keeps_its_scope(#[future] test_env: TestEnv) {
    use crate::helpers::invoice_lines::line;
    use crate::helpers::invoices::{draft, pay_amount};
    use meteroid_store::domain::LineItem;

    let env = test_env.await;
    let a = subscription()
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("wallet");
    let pack = env
        .store()
        .fund_scoped_customer_credit_wallet(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 5_000,
                classification: GrantClassification::Paid,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "A's pack".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            GrantScope {
                subscription_id: Some(a),
                application: CreditApplication::Usage,
                product_ids: Vec::new(),
            },
        )
        .await
        .expect("A's pack")
        .grant_id;
    let of_a = |line: LineItem| LineItem {
        subscription_id: Some(a),
        ..line
    };
    let drafted = draft(
        &env.setup,
        vec![
            of_a(line("Usage", 2_000, true)),
            of_a(line("Seats", 500, false)),
        ],
    )
    .await
    .expect("draft");
    let finalized = env
        .services()
        .finalize_invoice(Actor::System, drafted.id, TENANT_ID)
        .await
        .expect("finalize")
        .invoice;
    assert_eq!(
        (finalized.applied_credits, finalized.amount_due),
        (2_000, 500)
    );
    pay_amount(&env.setup, drafted.id, 500).await.expect("paid");
    let local_id = |name: &str| {
        finalized
            .line_items
            .iter()
            .find(|l| l.name == name)
            .unwrap()
            .local_id
            .clone()
    };
    let note = |local_id: String, amount_cents: i64| CreateCreditNoteParams {
        invoice_id: drafted.id,
        line_items: vec![CreditLineItem::Amount {
            local_id,
            amount_cents,
        }],
        reason: Some("Credited".into()),
        memo: None,
        credit_type: CreditType::CreditToBalance,
        custom_properties: None,
    };

    env.store()
        .create_and_finalize_credit_note(Actor::System, TENANT_ID, note(local_id("Usage"), 1_000))
        .await
        .expect("first note");
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, pack).await,
        4_000,
        "back to the grant that paid it, scope intact"
    );

    cancel_on(&env, a, CancellationEffectiveAt::Immediate).await;
    env.process_due_events().await;
    assert!(grant_row(&env, CUST_UBER_ID, pack).await.ends_at.is_some());

    env.store()
        .create_and_finalize_credit_note(Actor::System, TENANT_ID, note(local_id("Usage"), 1_000))
        .await
        .expect("second note");
    let movements = entries(&env, pack).await;
    assert!(
        movements
            .last()
            .is_some_and(|last| last.starts_with("DRAW_REVERSAL 1000")),
        "inside the settlement grace, the sweep lapses it: {movements:?}"
    );
    assert_eq!(
        grant_remaining(env.store(), TENANT_ID, CUST_UBER_ID, pack).await,
        5_000
    );
    env.store()
        .expire_customer_credit_for(TENANT_ID, CUST_UBER_ID, Utc::now())
        .await
        .expect("the sweep after the grace");
    let movements = entries(&env, pack).await;
    assert_eq!(
        movements.last(),
        Some(&format!("EXPIRY -5000 {ENDED_WITH_SUBSCRIPTION}")),
        "the ended grant lapses whole at the sweep: {movements:?}"
    );
    env.store()
        .create_and_finalize_credit_note(Actor::System, TENANT_ID, note(local_id("Seats"), 500))
        .await
        .expect("third note");
    let minted: Vec<_> = grant_rows(&env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.source_kind == "CREDIT_NOTE")
        .collect();
    assert_eq!(minted.len(), 1);
    assert_eq!((minted[0].amount, minted[0].subscription_id), (500, None));
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// T5 (`once`). Two subscriptions renewing on one consolidated invoice: a welcome credit added on
/// the first day is billed on February's parent, scoped to its subscription, never on March's.
#[rstest]
#[tokio::test]
async fn a_once_credit_billed_on_a_consolidated_invoice_is_never_billed_again(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::subscription_amendment::{
        AddExtraComponent, AddOnChanges, ComponentChanges, SubscriptionAmendment,
    };
    use meteroid_store::domain::subscription_changes::PlanChangeMode;

    let env = test_env.await;
    env.set_consolidate_recurring_invoices(true).await;
    let first_day = date(2024, 1, 1);
    let welcomed = start(&env, CUST_UBER_ID, PLAN_VERSION_STARTER_ID, first_day).await;
    start(&env, CUST_UBER_ID, PLAN_VERSION_STARTER_ID, first_day).await;
    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            welcomed,
            TENANT_ID,
            SubscriptionAmendment {
                apply_mode: PlanChangeMode::Immediate,
                component_changes: ComponentChanges {
                    edited: vec![],
                    added: vec![AddExtraComponent {
                        name: "Welcome credit".to_owned(),
                        product: new_pack_product("Welcome credit"),
                        price_entry: PriceEntry::New(PriceInput {
                            cadence: BillingPeriodEnum::Monthly,
                            currency: "EUR".to_owned(),
                            pricing: Pricing::Credit {
                                unit_price: Decimal::ZERO,
                                quantity: 1,
                                credit_amount: dec!(20),
                                expiry: None,
                                credit_unit_id: None,
                                rollover: None,
                                once: true,
                            },
                            credit_unit: None,
                        }),
                    }],
                    removed: vec![],
                },
                add_on_changes: AddOnChanges::default(),
            },
            first_day,
        )
        .await
        .expect("amendment applied");

    env.process_cycles().await;
    env.process_cycles().await;

    let welcome_lines = |invoices: &[Invoice]| {
        invoices
            .iter()
            .filter(|invoice| invoice.subscription_id.is_none())
            .flat_map(|invoice| invoice.line_items.iter())
            .filter(|line| line.kind == LineItemKind::Credit && line.name == "Welcome credit")
            .count()
    };
    let invoices = env.get_customer_invoices(CUST_UBER_ID).await;
    let parents = invoices
        .iter()
        .filter(|invoice| invoice.subscription_id.is_none())
        .count();
    assert_eq!(
        parents, 2,
        "February and March renew on consolidated invoices"
    );
    assert_eq!(
        welcome_lines(&invoices),
        1,
        "billed on February's parent only"
    );
    let welcome: Vec<_> = grant_rows(&env, CUST_UBER_ID)
        .await
        .into_iter()
        .filter(|g| g.source_kind == "INCLUDED_CREDIT" && g.amount == 2_000)
        .collect();
    assert_eq!(welcome.len(), 1);
    assert_eq!(welcome[0].subscription_id, Some(welcomed));
}

/// A cancellation and a scheduled plan change of the same subscription, both waiting on its
/// customer, run one after the other: each takes the customer first, so neither deadlocks.
#[rstest]
#[tokio::test]
async fn a_cancellation_racing_a_scheduled_plan_change_never_deadlocks(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;

    let env = test_env.await;
    for round in 0..4 {
        let sub_id = subscription()
            .plan_version(PLAN_VERSION_STARTER_ID)
            .on_start()
            .no_trial()
            .create(env.services())
            .await;
        let mut holder = env.conn().await;
        diesel::sql_query("BEGIN")
            .execute(&mut holder)
            .await
            .expect("begin");
        diesel::sql_query("SELECT id FROM customer WHERE id = $1 FOR NO KEY UPDATE")
            .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
            .execute(&mut holder)
            .await
            .expect("customer locked");
        let changing = async {
            tokio::time::sleep(std::time::Duration::from_millis(if round % 2 == 0 {
                100
            } else {
                300
            }))
            .await;
            env.services()
                .schedule_plan_change(
                    Actor::System,
                    sub_id,
                    TENANT_ID,
                    PLAN_VERSION_PRO_ID,
                    vec![],
                )
                .await
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        };
        let cancelling = async {
            tokio::time::sleep(std::time::Duration::from_millis(if round % 2 == 0 {
                300
            } else {
                100
            }))
            .await;
            env.services()
                .cancel_subscription(
                    Actor::System,
                    sub_id,
                    TENANT_ID,
                    None,
                    CancellationEffectiveAt::EndOfBillingPeriod,
                )
                .await
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        };
        let releasing = async {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            diesel::sql_query("COMMIT")
                .execute(&mut holder)
                .await
                .expect("commit");
        };
        let (changed, cancelled, ()) = tokio::join!(changing, cancelling, releasing);
        for outcome in [&changed, &cancelled] {
            if let Err(error) = outcome {
                assert!(
                    !error.to_lowercase().contains("deadlock"),
                    "round {round}: {error}"
                );
            }
        }
        assert!(cancelled.is_ok(), "round {round}: {cancelled:?}");
    }
}

async fn starter_minimum(env: &TestEnv, amount_subunit: i64) {
    use diesel_async::RunQueryDsl;
    use diesel_models::plan_version_adjustments::PlanVersionAdjustmentRowNew;
    diesel::insert_into(diesel_models::schema::plan_version_adjustment::table)
        .values(&PlanVersionAdjustmentRowNew {
            kind: diesel_models::enums::AdjustmentKindEnum::Minimum,
            plan_version_id: PLAN_VERSION_STARTER_ID,
            amount_subunit,
            includes_all_components: true,
        })
        .execute(&mut env.conn().await)
        .await
        .expect("minimum");
}

/// Starter (29.00 fee + one 10.00 seat) anchored on the 1st, started on `start`.
async fn starter_anchored_on_the_first(env: &TestEnv, start: NaiveDate) -> SubscriptionId {
    let mut request = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .request();
    request.subscription.billing_day_anchor = Some(1);
    env.services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("subscription")
        .id
}

fn note_lines(note: &meteroid_store::domain::credit_notes::CreditNote) -> Vec<(String, i64)> {
    let mut lines: Vec<_> = note
        .line_items
        .iter()
        .map(|line| (line.name.clone(), line.amount_subtotal))
        .collect();
    lines.sort();
    lines
}

async fn notes_of(
    env: &TestEnv,
    invoice: &Invoice,
) -> Vec<meteroid_store::domain::credit_notes::CreditNote> {
    env.store()
        .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("credit notes")
}

fn shortfall_lines(invoice: &Invoice) -> Vec<(i64, NaiveDate, NaiveDate)> {
    invoice
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::MinimumShortfall)
        .map(|line| (line.amount_subtotal, line.start_date, line.end_date))
        .collect()
}

fn invoice_starting(invoices: &[Invoice], start: NaiveDate) -> Invoice {
    invoices
        .iter()
        .find(|invoice| invoice.billing_period_start == start)
        .unwrap_or_else(|| panic!("the invoice of {start}"))
        .clone()
}

/// A 1,000.00 minimum, started Sep 20 on a period anchored on the 1st: September's 11 days are
/// trued up on October's invoice. Cancelled on Oct 10, only October's fees come back (2900 × 22/31
/// = 2058, 1000 × 22/31 = 710); the final invoice trues up 100000 × 9/31 = 29032 less 842 + 290.
#[rstest]
#[tokio::test]
async fn a_cancellation_trues_up_the_days_served_on_the_final_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    starter_minimum(&env, 100_000).await;
    let sub_id = starter_anchored_on_the_first(&env, date(2024, 9, 20)).await;
    env.process_cycles().await;
    assert_eq!(
        env.get_subscription(sub_id).await.current_period_start,
        date(2024, 10, 1)
    );
    let invoices = env.get_invoices(sub_id).await;
    let september = invoice_starting(&invoices, date(2024, 9, 20));
    let october = invoice_starting(&invoices, date(2024, 10, 1));
    assert!(
        shortfall_lines(&september).is_empty(),
        "no period has completed on the first invoice"
    );
    let september_fees: i64 = september
        .line_items
        .iter()
        .filter(|line| line.kind == LineItemKind::Regular)
        .map(|line| line.amount_subtotal)
        .sum();
    assert_eq!(
        shortfall_lines(&october),
        vec![(
            36_667 - september_fees,
            date(2024, 9, 20),
            date(2024, 10, 1)
        )],
        "100000 × 11/30, less what September's 11 days billed"
    );

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 10, 10)),
    )
    .await;
    env.process_due_events().await;

    assert!(
        notes_of(&env, &september).await.is_empty(),
        "September is not credited"
    );
    let notes = notes_of(&env, &october).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(
        note_lines(&notes[0]),
        vec![
            ("Platform Fee".to_owned(), -2_058),
            ("Seats".to_owned(), -710),
        ],
        "a true-up is never credited"
    );
    let last = invoice_starting(&env.get_invoices(sub_id).await, date(2024, 10, 10));
    assert_eq!(
        shortfall_lines(&last),
        vec![(27_900, date(2024, 10, 1), date(2024, 10, 10))]
    );

    // The sweep visits the recorded cancellation, more than once: the fees are credited once.
    for _ in 0..2 {
        assert_eq!(
            env.services()
                .settle_missed_plan_change_endings(100)
                .await
                .expect("sweep"),
            0
        );
    }
    assert_eq!(notes_of(&env, &october).await.len(), 1);
}

/// The same cancellation on Oct 10 while October's invoice (no auto-advance) is still a draft: it
/// trues up September, and the final invoice already counts the fees' unused days its note will
/// credit, so it trues up 27900 whichever of the two is issued first.
#[rstest]
#[tokio::test]
async fn a_cancellation_during_a_draft_period_never_credits_the_minimum(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    starter_minimum(&env, 100_000).await;
    let mut request = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(date(2024, 9, 1))
        .on_start()
        .no_trial()
        .no_auto_advance()
        .request();
    request.subscription.billing_day_anchor = Some(1);
    let sub_id = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .expect("subscription")
        .id;
    env.process_cycles().await;
    let october = invoice_starting(&env.get_invoices(sub_id).await, date(2024, 10, 1));

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 10, 10)),
    )
    .await;
    env.process_due_events().await;
    assert!(notes_of(&env, &october).await.is_empty());
    let last = invoice_starting(&env.get_invoices(sub_id).await, date(2024, 10, 10));
    assert_eq!(
        shortfall_lines(&last),
        vec![(27_900, date(2024, 10, 1), date(2024, 10, 10))],
        "29032 less 842 + 290, before October's note is issued"
    );

    env.manual_finalize_invoice(october.id)
        .await
        .expect("finalized");
    for _ in 0..2 {
        env.services()
            .settle_missed_plan_change_endings(100)
            .await
            .expect("sweep");
    }

    let issued = env.get_detailed_invoice(october.id).await.invoice;
    assert_eq!(
        shortfall_lines(&issued),
        vec![(96_100, date(2024, 9, 1), date(2024, 10, 1))],
        "100000 less September's 2900 + 1000"
    );
    let notes = notes_of(&env, &october).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(
        note_lines(&notes[0]),
        vec![
            ("Platform Fee".to_owned(), -2_058),
            ("Seats".to_owned(), -710),
        ]
    );
    env.manual_finalize_invoice(last.id)
        .await
        .expect("final invoice issued");
    let last = env.get_detailed_invoice(last.id).await.invoice;
    assert_eq!(
        shortfall_lines(&last),
        vec![(27_900, date(2024, 10, 1), date(2024, 10, 10))],
        "the same once October's note is issued"
    );
}

/// Starter plus a quarterly 300.00 pack and a 1,000.00 minimum, started Jul 1. The pack widens how
/// far back the cancellation looks for October's lines (three months), but no true-up is ever
/// credited: on Oct 10, October's fees by 22/31 and the unpaid pack by 83 of its 92 days (30000 ×
/// 83/92 = 27065); the pack never counts toward the minimum.
#[rstest]
#[tokio::test]
async fn a_quarterly_pack_never_reaches_back_to_earlier_minimums(#[future] test_env: TestEnv) {
    let env = test_env.await;
    starter_minimum(&env, 100_000).await;
    use meteroid_store::repositories::price_components::PriceComponentInterface;
    crate::helpers::plans::as_draft(
        env.store(),
        PLAN_VERSION_STARTER_ID,
        env.store().create_price_component_from_ref(
            "Quarterly credit".to_owned(),
            new_pack_product("Quarterly credit"),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Quarterly,
                currency: "EUR".to_owned(),
                pricing: Pricing::Credit {
                    unit_price: dec!(300),
                    quantity: 1,
                    credit_amount: dec!(300),
                    expiry: Some(CreditExpiry::BillingPeriod),
                    credit_unit_id: None,
                    rollover: None,
                    once: false,
                },
                credit_unit: None,
            })],
            PLAN_VERSION_STARTER_ID,
            TENANT_ID,
        ),
    )
    .await
    .expect("quarterly pack");
    let sub_id = starter_anchored_on_the_first(&env, date(2024, 7, 1)).await;
    for _ in 0..3 {
        env.process_cycles().await;
    }
    assert_eq!(
        env.get_subscription(sub_id).await.current_period_start,
        date(2024, 10, 1)
    );
    let invoices = env.get_invoices(sub_id).await;
    let october = invoices
        .iter()
        .find(|i| i.billing_period_start == date(2024, 10, 1))
        .expect("October's invoice")
        .clone();

    cancel_on(
        &env,
        sub_id,
        CancellationEffectiveAt::Date(date(2024, 10, 10)),
    )
    .await;
    env.process_due_events().await;

    for earlier in invoices.iter().filter(|i| i.id != october.id) {
        assert!(
            notes_of(&env, earlier).await.is_empty(),
            "{} is not credited",
            earlier.billing_period_start
        );
    }
    let notes = notes_of(&env, &october).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(
        note_lines(&notes[0]),
        vec![
            ("Platform Fee".to_owned(), -2_058),
            ("Quarterly credit".to_owned(), -27_065),
            ("Seats".to_owned(), -710),
        ]
    );
    assert_eq!(
        shortfall_lines(&october),
        vec![(96_100, date(2024, 9, 1), date(2024, 10, 1))],
        "September trued up on 2900 + 1000, the pack left out"
    );
    let last = invoice_starting(&env.get_invoices(sub_id).await, date(2024, 10, 10));
    assert_eq!(
        shortfall_lines(&last),
        vec![(27_900, date(2024, 10, 1), date(2024, 10, 10))]
    );
}

/// An immediate cancellation on the day the period renewed credits the whole period: Scale's
/// 99.00 + 25.00 fees and its 500.00 pack, whose paid credit is untouched (today's 10.00 of usage
/// is paid by the bonus first). The paid credit is reversed with the note; none of it lapses.
#[tokio::test]
async fn an_immediate_cancellation_on_the_renewal_day_credits_the_whole_period() {
    let (env, usage) = usage_env().await;
    halyard(&env, CreditExpiry::BillingPeriod).await;
    scale_usage(&env).await;
    let today = Utc::now().date_naive();
    // A monthly subscription renews on its start's day of the month, which a month ago may not
    // have (Oct 31 after Sep 30, Mar 29–31): start back as far as needed for it to renew today.
    let (cycles, previous) = (1..=12)
        .filter_map(|months| {
            today
                .checked_sub_months(chrono::Months::new(months))
                .map(|start| (months, start))
        })
        .find(|(_, start)| start.day() == today.day())
        .expect("a past month with today's day");
    let sub_id = start(&env, CUST_UBER_ID, SCALE, previous).await;
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: previous.and_time(NaiveTime::MIN),
                end: today.and_time(NaiveTime::MIN),
            },
        },
    );
    pay(&env, &invoice(&env, sub_id, 0).await).await;
    // One renewal per pass.
    for _ in 0..cycles {
        env.process_cycles().await;
        env.process_due_events().await;
    }
    let renewal = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|i| i.billing_period_start == today)
        .expect("today's renewal");
    let renewal = env
        .services()
        .finalize_invoice(Actor::System, renewal.id, TENANT_ID)
        .await
        .expect("finalized")
        .invoice;
    pay(&env, &renewal).await;

    cancel_on(&env, sub_id, CancellationEffectiveAt::Immediate).await;
    env.process_due_events().await;

    let notes = notes_of(&env, &renewal).await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].credit_type, CreditType::CreditToBalance);
    assert_eq!(
        note_lines(&notes[0]),
        vec![
            ("Monthly credit".to_owned(), -50_000),
            ("Platform Fee".to_owned(), -9_900),
            ("Seats".to_owned(), -2_500),
        ]
    );
    assert_eq!(notes[0].subtotal, -62_400);
    let rows = grant_rows(&env, CUST_UBER_ID).await;
    let paid = rows
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PAID"
                && g.invoice_id == Some(renewal.id)
        })
        .expect("today's paid pack");
    let bonus = rows
        .iter()
        .find(|g| {
            g.source_kind == "CREDIT_PURCHASE"
                && g.classification == "PROMOTIONAL"
                && g.invoice_id == Some(renewal.id)
        })
        .expect("today's bonus");
    assert_eq!(
        entries(&env, paid.id).await,
        vec![
            "FUND 50000".to_owned(),
            format!("REVERSAL -50000 {CANCELLATION_REMOVAL}"),
        ],
        "no paid credit lapses"
    );
    assert_eq!(
        entries(&env, bonus.id).await,
        vec!["FUND 5000", "DRAW -1000"]
    );
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// The undo of a scheduled cancellation waits on the customer the cancellation holds while it
/// runs; once the cancellation has executed, the undo is refused and the event stays completed.
#[rstest]
#[tokio::test]
async fn undoing_a_cancellation_that_already_ran_is_refused(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct Event {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        id: uuid::Uuid,
        #[diesel(sql_type = diesel::sql_types::Text)]
        status: String,
    }
    let env = test_env.await;
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    cancel_on(&env, sub_id, CancellationEffectiveAt::EndOfBillingPeriod).await;
    async fn event(conn: &mut diesel_models::PgConn, sub_id: SubscriptionId) -> Event {
        diesel::sql_query(
            "SELECT id, status::text AS status FROM scheduled_event
             WHERE subscription_id = $1 AND event_type = 'CANCEL_SUBSCRIPTION'",
        )
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .get_result::<Event>(conn)
        .await
        .expect("the cancellation")
    }
    let scheduled = event(&mut env.conn().await, sub_id).await;
    assert_eq!(scheduled.status, "PENDING");

    // Stands in for the lifecycle worker: it holds the customer while it executes the event.
    let mut worker = env.conn().await;
    diesel::sql_query("BEGIN")
        .execute(&mut worker)
        .await
        .expect("begin");
    diesel::sql_query("SELECT id FROM customer WHERE id = $1 FOR NO KEY UPDATE")
        .bind::<diesel::sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
        .execute(&mut worker)
        .await
        .expect("customer locked");
    let undoing = async {
        env.services()
            .cancel_scheduled_event(
                Actor::System,
                common_domain::ids::ScheduledEventId::from(scheduled.id),
                sub_id,
                TENANT_ID,
            )
            .await
    };
    let executing = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        diesel::sql_query("UPDATE scheduled_event SET status = 'COMPLETED' WHERE id = $1")
            .bind::<diesel::sql_types::Uuid, _>(scheduled.id)
            .execute(&mut worker)
            .await
            .expect("executed");
        diesel::sql_query("COMMIT")
            .execute(&mut worker)
            .await
            .expect("commit");
    };
    let (undone, ()) = tokio::join!(undoing, executing);
    assert!(undone.is_err(), "an executed cancellation cannot be undone");
    assert_eq!(
        event(&mut env.conn().await, sub_id).await.status,
        "COMPLETED"
    );
}

/// A fixed-term subscription completing on Feb 1: its included credit ends then, the day it also
/// expires, so it lapses "Ended with subscription"; the final invoice takes over the 40.00 of
/// January's usage it settled, and the other 60.00 lapse.
#[tokio::test]
async fn a_completed_subscription_ends_its_credit_on_its_end_date() {
    let (env, usage) = usage_env().await;
    included_credit_plan(&env, true).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(LARGE)
        .start_date(date(2024, 1, 1))
        .end_date(date(2024, 2, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    set_usage(&usage, date(2024, 1, 1), date(2024, 2, 1), 400);

    // The renewal on Feb 1 stops at the end date; the next transition completes it.
    env.process_cycles().await;
    env.process_cycles().await;
    env.process_due_events().await;

    assert_eq!(
        env.get_subscription(sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::Completed
    );
    let pack = plan_credit(&grant_rows(&env, CUST_UBER_ID).await, "INCLUDED_CREDIT").clone();
    assert_eq!(pack.ends_at, Some(midnight(date(2024, 2, 1))));
    assert_eq!(pack.ends_at, pack.expires_at);
    assert_eq!(pack.end_reason.as_deref(), Some("SUBSCRIPTION_ENDED"));
    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(
        money(terminal_invoice(&invoices, date(2024, 2, 1))),
        (4_000, 4_000, 0, 0, 0, 0)
    );

    sweep(
        &env,
        CUST_UBER_ID,
        midnight(date(2024, 2, 1)) + Duration::hours(1),
    )
    .await;
    assert_eq!(
        entries(&env, pack.id).await,
        vec![
            "FUND 10000".to_owned(),
            "DRAW -4000".to_owned(),
            format!("EXPIRY -6000 {ENDED_WITH_SUBSCRIPTION}"),
        ]
    );
    let expired = expired_events(&env, pack.id).await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].reason, GrantLapseReason::SubscriptionEnded);
    assert_ledger_balances(env.store(), TENANT_ID, CUST_UBER_ID).await;
}

/// A subscription that never started (waiting for checkout, starting next month) holding a grant
/// of its own is aborted: the grant ends when it was aborted, not at the later start.
#[rstest]
#[tokio::test]
async fn an_aborted_subscription_ends_its_credit_when_aborted(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    let later = Utc::now().date_naive() + Duration::days(20);
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(later)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    let wallet = env
        .store()
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .expect("wallet");
    let grant = env
        .store()
        .fund_scoped_customer_credit_wallet(
            FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet.id,
                amount_minor: 2_000,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Signup offer".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            GrantScope {
                subscription_id: Some(sub_id),
                application: CreditApplication::Usage,
                product_ids: Vec::new(),
            },
        )
        .await
        .expect("a grant of the subscription")
        .grant_id;

    let before = Utc::now();
    cancel_on(&env, sub_id, CancellationEffectiveAt::Immediate).await;
    let after = Utc::now();

    assert_eq!(
        env.get_subscription(sub_id).await.status,
        diesel_models::enums::SubscriptionStatusEnum::Aborted
    );
    let ended = grant_row(&env, CUST_UBER_ID, grant).await;
    let ends_at = ended.ends_at.expect("ended with the subscription");
    assert!(
        before <= ends_at && ends_at <= after,
        "ended when aborted ({ends_at}), not at the start on {later}"
    );
    assert_eq!(ended.end_reason.as_deref(), Some("SUBSCRIPTION_ENDED"));
}
