//! Payment reminder scheduling: enrolment, due execution, holds, lifecycle hooks, per-invoice
//! actions and the reminder-specific worker checks. Every sweep runs with
//! an explicit `now`; the base date sits in the past so the worker's real clock never sees a
//! pause or cooldown the scenario did not ask for.

use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};
use http::HeaderMap;
use rstest::rstest;
use serde_json::json;

use common_domain::ids::{BaseId, CustomerId, InvoiceId, PaymentReminderSequenceId};
use diesel_models::entity_activity::EntityActivityRow;
use diesel_models::enums::{
    InvoicePaymentStatus, ReminderEnrolledViaEnum, ReminderNotEnrolledReasonEnum,
    ReminderStateEnum, ReminderStopReasonEnum,
};
use diesel_models::invoices::InvoiceRow;
use diesel_models::payment_reminders::InvoicePaymentReminderRow;
use diesel_models::query::sent_email::SentEmailFilter;
use diesel_models::sent_email::SentEmailRow;
use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid_mailer::model::Email;
use meteroid_mailer::render::render;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::InvoicingEntityPatch;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::connectors::{
    Connector, ConnectorConfig, CredentialState, MockChargeBehavior, MockPublicData,
    MockSensitiveData, PaymentConnectorConfig, PaymentProviderConfig,
};
use meteroid_store::domain::customer_emails::{EmailContext, EmailKind, EmailSettingsPatch};
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::payment_reminders::{
    EmailCopy, EmailCopyByLocale, NotEligibleReason, PaymentReminderSequence,
    PaymentReminderSequenceNew, PaymentReminderSequencePatch, PaymentReminderStepInput,
    ReminderEnrolledVia,
};
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditType};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{
    CreditNoteInterface, CustomerEmailsInterface, InvoiceInterface, PaymentRemindersInterface,
};
use meteroid_store::services::customer_emails::{BuildOutcome, SendOrigin};
use meteroid_store::services::payment_reminders::sweep::{
    ENROLL_BATCH, SweepStats, manual_reminder_dedupe_key, reminder_dedupe_key,
};

use crate::data::ids::*;
use crate::harness::reminders::{ENTITY_LOCALE, FinalizedInvoice, audience, steps};
use crate::harness::{TestEnv, subscription, test_env};

const ACTOR: Actor = Actor::User { id: USER_ID };
const RECIPIENT: &str = "billing@uber.example";

// ---- clock ---------------------------------------------------------------------------------

/// Monday 2026-06-15; the entity has no timezone, so the send window is 09:00–15:00 UTC.
fn base() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 15).expect("base date")
}

fn at(day: i64, hour: u32, minute: u32) -> NaiveDateTime {
    (base() + Duration::days(day))
        .and_hms_opt(hour, minute, 0)
        .expect("time")
}

/// Midnight of `day`, the way `due_at` is stored.
fn due(day: i64) -> NaiveDateTime {
    at(day, 0, 0)
}

fn fire(day: i64) -> NaiveDateTime {
    at(day, 9, 0)
}

// ---- fixtures ------------------------------------------------------------------------------

/// An open invoice of `customer` due on `due_day` (finalized on day 0 at 08:00), with a PDF.
async fn open_invoice(env: &TestEnv, customer: CustomerId, due_day: i64) -> InvoiceId {
    env.finalized_invoice_with_pdf(
        FinalizedInvoice::one_off(customer).dates(due(due_day), at(0, 8, 0)),
    )
    .await
}

/// Reminders on for the tenant and the entity, the default sequence enabled with `offsets`.
async fn default_with_steps(env: &TestEnv, offsets: &[i32]) -> PaymentReminderSequence {
    env.enable_reminders().await;
    let default = env.default_sequence(INVOICING_ENTITY_ID, true).await;
    env.set_sequence_steps(default.id, offsets).await
}

fn customer_audience(customer: CustomerId) -> meteroid_store::domain::payment_reminders::Audience {
    audience(json!([{ "customer": { "customer_ids": [customer] } }]))
}

async fn row(env: &TestEnv, invoice: InvoiceId) -> InvoicePaymentReminderRow {
    env.reminder_row(invoice)
        .await
        .unwrap_or_else(|| panic!("no reminder row for {invoice}"))
}

async fn set_status(env: &TestEnv, invoice: InvoiceId, status: InvoicePaymentStatus) {
    let mut conn = env.conn().await;
    InvoiceRow::apply_payment_status(&mut conn, invoice, TENANT_ID, status, None)
        .await
        .expect("payment status");
}

async fn set_auto_charge(env: &TestEnv, invoice: InvoiceId, at_: Option<NaiveDateTime>) {
    let mut conn = env.conn().await;
    InvoiceRow::set_auto_charge_requested(&mut conn, invoice, TENANT_ID, at_)
        .await
        .expect("auto charge");
}

async fn activities(env: &TestEnv, invoice: InvoiceId) -> Vec<EntityActivityRow> {
    let mut conn = env.conn().await;
    EntityActivityRow::list_by_entity(
        &mut conn,
        TENANT_ID,
        "invoice",
        invoice.as_uuid(),
        None,
        100,
    )
    .await
    .expect("activities")
}

async fn reminder_activities(env: &TestEnv, invoice: InvoiceId) -> Vec<EntityActivityRow> {
    activities(env, invoice)
        .await
        .into_iter()
        .filter(|a| a.activity_type.starts_with("payment_reminder."))
        .collect()
}

fn of_type<'a>(rows: &'a [EntityActivityRow], activity_type: &str) -> Vec<&'a EntityActivityRow> {
    rows.iter()
        .filter(|a| a.activity_type == activity_type)
        .collect()
}

fn meta<'a>(row: &'a EntityActivityRow, key: &str) -> Option<&'a serde_json::Value> {
    row.metadata.as_ref().and_then(|m| m.get(key))
}

/// Every message on the email queue, visible or not: `(dedupe_key, total retry attempts)`.
async fn queued(env: &TestEnv) -> Vec<(String, u32)> {
    use diesel_async::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Jsonb)]
        message: serde_json::Value,
    }
    let mut conn = env.conn().await;
    diesel::sql_query("SELECT message FROM pgmq.q_send_email_request ORDER BY msg_id")
        .load::<Row>(&mut conn)
        .await
        .expect("read queue")
        .into_iter()
        .map(|r| {
            let m = &r.message["CustomerEmail"];
            (
                m["dedupe_key"].as_str().unwrap_or_default().to_string(),
                m["attempts"]
                    .as_object()
                    .map_or(0, |a| a.values().filter_map(|v| v.as_u64()).sum::<u64>())
                    as u32,
            )
        })
        .collect()
}

async fn sent_rows(env: &TestEnv) -> Vec<SentEmailRow> {
    let mut conn = env.conn().await;
    SentEmailRow::list(&mut conn, TENANT_ID, &SentEmailFilter::default(), None, 100)
        .await
        .expect("sent rows")
        .into_iter()
        .map(|r| r.email)
        .collect()
}

async fn sql(env: &TestEnv, statement: &str) {
    use diesel_async::RunQueryDsl;
    let mut conn = env.conn().await;
    diesel::sql_query(statement)
        .execute(&mut conn)
        .await
        .expect("sql");
}

fn assert_failed_precondition<T: std::fmt::Debug>(
    result: Result<T, error_stack::Report<StoreError>>,
    needle: &str,
) {
    match result {
        Err(e) => match e.current_context() {
            StoreError::FailedPrecondition(msg) => {
                assert!(msg.contains(needle), "expected {needle:?} in {msg:?}")
            }
            other => panic!("expected FailedPrecondition, got {other:?}"),
        },
        Ok(v) => panic!("expected FailedPrecondition, got {v:?}"),
    }
}

fn mock_connector() -> Connector {
    Connector {
        status: meteroid_store::domain::enums::ConnectorStatusEnum::Active,
        credentials: CredentialState::Unverified,
        webhook_route: None,
        id: MOCK_CONNECTOR_ID,
        created_at: Utc::now().naive_utc(),
        tenant_id: TENANT_ID,
        alias: "mock-payment-provider".to_string(),
        config: ConnectorConfig::Payment(PaymentConnectorConfig::configured(
            PaymentProviderConfig::Mock(MockPublicData::default(), MockSensitiveData {}),
        )),
    }
}

async fn deliver_webhook(env: &TestEnv, payload: String) {
    let connector = mock_connector();
    let connector_impl = initialize_payment_connector(&connector).expect("mock connector");
    let event = connector_impl
        .parse_event(payload.as_bytes(), &HeaderMap::new())
        .expect("parse_event")
        .expect("an event");
    handle_normalized_event(
        event,
        &connector,
        connector_impl.as_ref(),
        env.store().clone(),
        env.services(),
    )
    .await
    .expect("dispatcher handled the event");
    env.run_outbox_and_orchestration().await;
}

// ---- enrolment -----------------------------------------------------------------------------

/// Invoices a replay of past billing issued are neither enrolled nor reminded.
#[rstest]
#[tokio::test]
async fn invoices_held_from_delivery_are_never_reminded(#[future] test_env: TestEnv) {
    use diesel_models::replay_holds::hold_invoice_delivery;
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let held_before = open_invoice(&env, CUST_UBER_ID, 10).await;
    let held_after = open_invoice(&env, CUST_SPOTIFY_ID, 10).await;
    env.drain_email_queue().await;
    let mut conn = env.conn().await;
    hold_invoice_delivery(&mut conn, TENANT_ID, held_before)
        .await
        .expect("hold");

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats.enrolled, 1, "{stats:?}");
    assert!(env.reminder_row(held_before).await.is_none());
    assert_eq!(
        row(&env, held_after).await.state,
        ReminderStateEnum::Enrolled
    );

    hold_invoice_delivery(&mut conn, TENANT_ID, held_after)
        .await
        .expect("hold");
    assert_eq!(env.run_sweep(at(10, 10, 0)).await.executed, 0);
    assert_eq!(row(&env, held_after).await.last_offset_days, None);
    assert!(queued(&env).await.is_empty(), "no reminder is sent");
}

#[rstest]
#[tokio::test]
async fn enrolls_bank_transfer_and_manual_invoices_into_default_when_enabled(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.enable_reminders().await;
    env.seed_bank_account().await;
    let default = env.default_sequence(INVOICING_ENTITY_ID, true).await;
    assert_eq!(
        default
            .steps
            .iter()
            .map(|s| s.offset_days)
            .collect::<Vec<_>>(),
        vec![-3, 0, 7, 14]
    );

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .payment_methods_config(PaymentMethodsConfig::BankTransfer { account_id: None })
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    let bank = env.get_invoices(sub_id).await[0].clone();
    assert!(
        bank.auto_charge_requested_at.is_none(),
        "a bank transfer is a pay-link invoice"
    );
    let manual = env
        .finalized_invoice_with_pdf(FinalizedInvoice::one_off(CUST_SPOTIFY_ID))
        .await;
    env.drain_email_queue().await;

    let stats = env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!(stats.enrolled, 2, "{stats:?}");
    for invoice in [bank.id, manual] {
        let r = row(&env, invoice).await;
        assert_eq!(r.state, ReminderStateEnum::Enrolled);
        assert_eq!(r.sequence_id, Some(default.id));
        assert_eq!(r.enrolled_via, Some(ReminderEnrolledViaEnum::Default));
        assert_eq!(r.last_offset_days, None);
        assert_eq!(r.generation, 1);
        assert!(r.next_reminder_at.is_some());
        let acts = activities(&env, invoice).await;
        let enrolled = of_type(&acts, "payment_reminder.enrolled");
        assert_eq!(enrolled.len(), 1);
        assert_eq!(meta(enrolled[0], "via"), Some(&json!("DEFAULT")));
    }
    assert!(queued(&env).await.is_empty(), "enrolment never sends");
}

#[rstest]
#[tokio::test]
async fn nothing_enrolls_while_every_sequence_is_disabled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.enable_reminders().await;
    let default = env.default_sequence(INVOICING_ENTITY_ID, false).await;
    assert!(!default.enabled);
    env.create_sequence(INVOICING_ENTITY_ID, &[0, 7], false)
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats, SweepStats::default());
    assert!(env.reminder_row(invoice).await.is_none());
    assert!(reminder_activities(&env, invoice).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn auto_charged_invoice_not_enrolled_until_errored(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    default_with_steps(&env, &[0, 7]).await;

    let card = open_invoice(&env, CUST_UBER_ID, 10).await;
    set_auto_charge(&env, card, Some(at(0, 8, 0))).await;
    let parent = open_invoice(&env, CUST_COMODO_ID, 10).await;
    set_auto_charge(&env, parent, Some(at(0, 8, 0))).await;
    let child = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_COMODO_ID)
                .dates(due(10), at(0, 8, 0))
                .consolidated_into(parent),
        )
        .await;
    // Spotify has a saved card, but this invoice was never auto-charged: it is a pay-link one.
    let pay_link = open_invoice(&env, CUST_SPOTIFY_ID, 10).await;

    let stats = env.run_sweep(at(1, 10, 0)).await;
    assert_eq!(stats.enrolled, 1, "{stats:?}");
    assert!(env.reminder_row(card).await.is_none(), "auto-charged");
    assert!(
        env.reminder_row(parent).await.is_none(),
        "auto-charged parent"
    );
    assert!(
        env.reminder_row(child).await.is_none(),
        "consolidated child"
    );
    assert_eq!(row(&env, pay_link).await.state, ReminderStateEnum::Enrolled);

    set_status(&env, card, InvoicePaymentStatus::Errored).await;
    let stats = env.run_sweep(at(2, 10, 0)).await;
    assert_eq!(stats.enrolled, 1, "{stats:?}");
    assert_eq!(row(&env, card).await.state, ReminderStateEnum::Enrolled);
    assert!(env.reminder_row(parent).await.is_none());
}

#[rstest]
#[tokio::test]
async fn first_matching_audience_wins_and_default_catches_the_rest(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[0, 7]).await;
    let uber = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Uber",
            customer_audience(CUST_UBER_ID),
            &[0],
            true,
        )
        .await;
    let big = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Big",
            audience(json!([{ "amount": { "op": "gte", "threshold": 5000, "currency": "EUR" } }])),
            &[0],
            true,
        )
        .await;
    let uber_invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    let spotify_invoice = open_invoice(&env, CUST_SPOTIFY_ID, 10).await;
    let comodo_invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_COMODO_ID)
                .amount(100, "EUR")
                .dates(due(10), at(0, 8, 0)),
        )
        .await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats.enrolled, 3, "{stats:?}");
    let r = row(&env, uber_invoice).await;
    assert_eq!(
        (r.sequence_id, r.enrolled_via),
        (Some(uber.id), Some(ReminderEnrolledViaEnum::Audience))
    );
    let r = row(&env, spotify_invoice).await;
    assert_eq!(
        (r.sequence_id, r.enrolled_via),
        (Some(big.id), Some(ReminderEnrolledViaEnum::Audience))
    );
    let r = row(&env, comodo_invoice).await;
    assert_eq!(
        (r.sequence_id, r.enrolled_via),
        (Some(default.id), Some(ReminderEnrolledViaEnum::Default))
    );
}

#[rstest]
#[tokio::test]
async fn disabled_sequence_is_skipped_and_falls_through(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[0, 7]).await;
    let enterprise = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Enterprise",
            customer_audience(CUST_UBER_ID),
            &[0, 30],
            false,
        )
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    env.run_sweep(at(0, 10, 0)).await;
    let r = row(&env, invoice).await;
    assert_eq!(
        r.sequence_id,
        Some(default.id),
        "the disabled sequence is skipped"
    );
    assert_eq!(r.enrolled_via, Some(ReminderEnrolledViaEnum::Default));

    env.set_sequence_enabled(enterprise.id, true).await;
    env.run_sweep(at(1, 10, 0)).await;
    let r = row(&env, invoice).await;
    assert_eq!(r.sequence_id, Some(default.id), "frozen at enrolment (I7)");
    assert_eq!(r.generation, 1);
}

#[rstest]
#[tokio::test]
async fn customer_override_beats_audiences(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let uber = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Uber",
            customer_audience(CUST_UBER_ID),
            &[0],
            true,
        )
        .await;
    let vip = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "VIP",
            customer_audience(CUST_COMODO_ID),
            &[0, 3],
            true,
        )
        .await;
    env.set_customer_reminder_preference(CUST_UBER_ID, false, Some(vip.id))
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    env.run_sweep(at(0, 10, 0)).await;
    let r = row(&env, invoice).await;
    assert_eq!(r.sequence_id, Some(vip.id));
    assert_ne!(r.sequence_id, Some(uber.id));
    assert_eq!(
        r.enrolled_via,
        Some(ReminderEnrolledViaEnum::CustomerOverride)
    );
}

#[rstest]
#[tokio::test]
async fn disabled_override_falls_through(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let uber = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Uber",
            customer_audience(CUST_UBER_ID),
            &[0],
            true,
        )
        .await;
    let vip = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "VIP",
            customer_audience(CUST_COMODO_ID),
            &[0, 3],
            false,
        )
        .await;
    env.set_customer_reminder_preference(CUST_UBER_ID, false, Some(vip.id))
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    env.run_sweep(at(0, 10, 0)).await;
    let r = row(&env, invoice).await;
    assert_eq!(
        r.sequence_id,
        Some(uber.id),
        "audiences resolve as if the override were unset"
    );
    assert_eq!(r.enrolled_via, Some(ReminderEnrolledViaEnum::Audience));
    assert!(env.reminder_row(invoice).await.is_some());
}

#[rstest]
#[tokio::test]
async fn archived_override_falls_through(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[0, 7]).await;
    let vip = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "VIP",
            customer_audience(CUST_COMODO_ID),
            &[0, 3],
            true,
        )
        .await;
    env.set_customer_reminder_preference(CUST_UBER_ID, false, Some(vip.id))
        .await;
    env.store()
        .archive_reminder_sequence(ACTOR, TENANT_ID, vip.id, at(0, 9, 0))
        .await
        .expect("archive");
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    env.run_sweep(at(0, 10, 0)).await;
    let r = row(&env, invoice).await;
    assert_eq!(r.sequence_id, Some(default.id));
    assert_eq!(r.enrolled_via, Some(ReminderEnrolledViaEnum::Default));
}

#[rstest]
#[tokio::test]
async fn opt_out_skips_and_stops_in_flight(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let in_flight = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(
        row(&env, in_flight).await.state,
        ReminderStateEnum::Enrolled
    );

    env.set_customer_reminder_preference(CUST_UBER_ID, true, None)
        .await;
    let r = row(&env, in_flight).await;
    assert_eq!(r.stop_reason, Some(ReminderStopReasonEnum::CustomerOptOut));
    assert!(r.stopped_at.is_some() && r.next_reminder_at.is_none());

    let later = open_invoice(&env, CUST_UBER_ID, 12).await;
    let stats = env.run_sweep(at(20, 10, 0)).await;
    assert_eq!(stats.enrolled + stats.executed, 0, "{stats:?}");
    assert!(env.reminder_row(later).await.is_none());
    assert!(queued(&env).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn opt_out_rechecked_under_enroll_lock(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    env.run_sweep(at(0, 10, 0)).await;
    let r = row(&env, invoice).await;

    // The flag flips without the preference path (no stop): the scan, the enrol re-check and
    // the worker all read it live.
    sql(
        &env,
        &format!(
            "UPDATE customer SET payment_reminders_opt_out = true WHERE id = '{}'",
            CUST_UBER_ID.as_uuid()
        ),
    )
    .await;
    let other = open_invoice(&env, CUST_UBER_ID, 12).await;
    let stats = env.run_sweep(at(10, 10, 0)).await;
    assert_eq!(stats.executed + stats.enrolled, 0, "{stats:?}");
    assert!(env.reminder_row(other).await.is_none());
    assert_eq!(
        row(&env, invoice).await.last_offset_days,
        None,
        "held, not burnt"
    );

    env.store()
        .enqueue_customer_email(
            TENANT_ID,
            EmailKind::PaymentReminder,
            EmailContext::PaymentReminder {
                invoice_id: invoice,
                sequence_id: r.sequence_id.expect("sequence"),
                offset_days: 0,
                generation: r.generation,
                manual: true,
            },
            manual_reminder_dedupe_key(invoice, "opt-out"),
        )
        .await
        .expect("enqueue");
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, invoice).await;
    let skipped = of_type(&acts, "payment_reminder.skipped");
    assert_eq!(skipped.len(), 1);
    assert_eq!(meta(skipped[0], "reason"), Some(&json!("opted_out")));
}

#[rstest]
#[tokio::test]
async fn consolidated_children_never_enroll(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let parent = open_invoice(&env, CUST_UBER_ID, 10).await;
    let child = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID)
                .dates(due(10), at(0, 8, 0))
                .consolidated_into(parent),
        )
        .await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats.enrolled, 1, "{stats:?}");
    assert!(row(&env, parent).await.next_reminder_at.is_some());
    assert!(env.reminder_row(child).await.is_none());
}

#[rstest]
#[tokio::test]
async fn audience_edit_does_not_move_frozen_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let uber = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Uber",
            customer_audience(CUST_UBER_ID),
            &[0, 7],
            true,
        )
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(row(&env, invoice).await.sequence_id, Some(uber.id));

    env.patch_sequence(PaymentReminderSequencePatch {
        id: uber.id,
        audience: Some(Some(customer_audience(CUST_COMODO_ID))),
        ..Default::default()
    })
    .await;
    let stats = env.run_sweep(at(1, 10, 0)).await;
    assert_eq!(stats.enrolled, 0);
    let r = row(&env, invoice).await;
    assert_eq!(r.sequence_id, Some(uber.id));
    assert_eq!(r.generation, 1);
}

#[rstest]
#[tokio::test]
async fn step_edit_rearms_in_flight_and_completed_invoices(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[0, 7]).await;

    // In flight: enrolled at the top, day 0 executed on the 10th, waiting for +7.
    let in_flight = open_invoice(&env, CUST_UBER_ID, 10).await;
    // Completed: a single step executed on the 10th.
    let completed_seq = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Spotify",
            customer_audience(CUST_SPOTIFY_ID),
            &[0],
            true,
        )
        .await;
    let completed = open_invoice(&env, CUST_SPOTIFY_ID, 10).await;
    // Long completed: a 170-day-old invoice that went through the same one-step ladder.
    let old = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_SPOTIFY_ID).dates(due(-170), at(-180, 8, 0)),
        )
        .await;
    env.enroll_invoice(old, completed_seq.id).await;
    {
        let mut conn = env.conn().await;
        diesel_models::query::payment_reminders::InvoiceReminderUpdates::advance(
            &mut conn,
            old,
            TENANT_ID,
            0,
            None,
            at(-170, 9, 0),
        )
        .await
        .expect("complete the old ladder");
    }
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    assert_eq!(row(&env, in_flight).await.last_offset_days, Some(0));
    assert_eq!(row(&env, in_flight).await.next_reminder_at, Some(fire(17)));
    assert!(row(&env, completed).await.completed_at.is_some());
    assert!(row(&env, old).await.completed_at.is_some());

    // Insert +3 between the cursor and the next step: pulled to `now`, +3 fires next.
    env.patch_sequence_at(
        PaymentReminderSequencePatch {
            id: default.id,
            steps: Some(steps(&[0, 3, 7])),
            ..Default::default()
        },
        at(14, 10, 0),
    )
    .await;
    assert_eq!(
        row(&env, in_flight).await.next_reminder_at,
        Some(at(14, 10, 0))
    );
    let stats = env.run_sweep(at(14, 10, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    let r = row(&env, in_flight).await;
    assert_eq!(r.last_offset_days, Some(3));
    assert_eq!(r.next_reminder_at, Some(fire(17)));

    // Move the last step earlier: +7 becomes +5, already elapsed, fires now.
    env.patch_sequence_at(
        PaymentReminderSequencePatch {
            id: default.id,
            steps: Some(steps(&[0, 3, 5])),
            ..Default::default()
        },
        at(16, 10, 0),
    )
    .await;
    let stats = env.run_sweep(at(16, 10, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    let r = row(&env, in_flight).await;
    assert_eq!(r.last_offset_days, Some(5));
    assert!(r.completed_at.is_some() && r.next_reminder_at.is_none());

    // Append +7 after completion: the recent invoice is re-armed and catches up, the 170-day-old
    // one stays completed (its new last step is far outside the enrolment window).
    env.patch_sequence_at(
        PaymentReminderSequencePatch {
            id: completed_seq.id,
            steps: Some(steps(&[0, 7])),
            ..Default::default()
        },
        at(17, 12, 0),
    )
    .await;
    let r = row(&env, completed).await;
    assert!(r.completed_at.is_none());
    assert_eq!(r.next_reminder_at, Some(at(17, 12, 0)));
    let old_row = row(&env, old).await;
    assert!(old_row.completed_at.is_some() && old_row.next_reminder_at.is_none());
    let stats = env.run_sweep(at(17, 12, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    let r = row(&env, completed).await;
    assert_eq!(r.last_offset_days, Some(7));
    assert!(r.completed_at.is_some());
    assert_eq!(queued(&env).await.len(), 5);
}

#[rstest]
#[tokio::test]
async fn sweep_never_sends_more_than_one_reminder_per_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[0, 7, 14, 21, 35]).await;
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(due(-30), at(-40, 8, 0)),
        )
        .await;
    // Enrolled at the top before the ladder went stale, then not looked at for 30 days.
    let r = env.enroll_invoice(invoice, default.id).await;
    assert_eq!(r.last_offset_days, None);
    {
        let mut conn = env.conn().await;
        diesel_models::query::payment_reminders::InvoiceReminderUpdates::defer(
            &mut conn,
            invoice,
            TENANT_ID,
            fire(-30),
        )
        .await
        .expect("defer");
    }

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    let messages = queued(&env).await;
    assert_eq!(
        messages,
        vec![(reminder_dedupe_key(invoice, r.generation, 21), 0)]
    );
    let r = row(&env, invoice).await;
    assert_eq!(r.last_offset_days, Some(21));
    assert_eq!(r.next_reminder_at, Some(fire(5)));
    let acts = activities(&env, invoice).await;
    let executed = of_type(&acts, "payment_reminder.step_executed");
    assert_eq!(executed.len(), 1);
    assert_eq!(
        meta(executed[0], "skipped_offsets"),
        Some(&json!([0, 7, 14]))
    );

    let stats = env.run_sweep(at(0, 11, 0)).await;
    assert_eq!(stats.executed, 0, "nothing more today");
}

#[rstest]
#[tokio::test]
async fn enrollment_does_not_fire_in_the_same_sweep(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 0).await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!((stats.enrolled, stats.executed), (1, 0), "{stats:?}");
    assert_eq!(row(&env, invoice).await.next_reminder_at, Some(fire(0)));
    assert!(queued(&env).await.is_empty());

    let stats = env.run_sweep(at(0, 10, 15)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    assert_eq!(queued(&env).await.len(), 1);
}

// ---- holds and catch-up --------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn catch_up_after_pause_sends_last_elapsed_step_at_next_send_slot(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7, 14]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;

    let state = env
        .store()
        .pause_invoice_reminders(ACTOR, TENANT_ID, invoice, at(20, 0, 0), at(5, 10, 0))
        .await
        .expect("pause");
    assert_eq!(
        state.paused_until,
        Some(fire(20)),
        "ceiled to the send slot"
    );

    assert_eq!(env.run_sweep(at(10, 10, 0)).await.executed, 0);
    assert_eq!(
        env.run_sweep(at(20, 3, 0)).await.executed,
        0,
        "still paused at 03:00"
    );
    // Expired, but 16:00 is outside the window: rescheduled to tomorrow's slot.
    let stats = env.run_sweep(at(20, 16, 0)).await;
    assert_eq!((stats.executed, stats.deferred), (0, 1), "{stats:?}");
    assert_eq!(row(&env, invoice).await.next_reminder_at, Some(fire(21)));

    let stats = env.run_sweep(at(21, 9, 30)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    let r = row(&env, invoice).await;
    assert_eq!(r.last_offset_days, Some(7), "only the last elapsed step");
    assert_eq!(r.next_reminder_at, Some(fire(24)));
    assert_eq!(queued(&env).await.len(), 1);
}

#[rstest]
#[tokio::test]
async fn disabled_sequence_holds_then_reenable_catches_up_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.enable_reminders().await;
    env.default_sequence(INVOICING_ENTITY_ID, false).await;
    let seq = env
        .create_sequence(INVOICING_ENTITY_ID, &[0, 7], true)
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(row(&env, invoice).await.sequence_id, Some(seq.id));

    env.set_sequence_enabled(seq.id, false).await;
    assert_eq!(env.run_sweep(at(10, 10, 0)).await.executed, 0);
    assert_eq!(env.run_sweep(at(17, 10, 0)).await.executed, 0);
    let r = row(&env, invoice).await;
    assert_eq!(
        (r.last_offset_days, r.next_reminder_at),
        (None, Some(fire(10))),
        "held"
    );

    env.set_sequence_enabled(seq.id, true).await;
    let stats = env.run_sweep(at(18, 10, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    assert_eq!(queued(&env).await.len(), 1);
    let r = row(&env, invoice).await;
    assert_eq!(r.last_offset_days, Some(7));
    assert!(r.completed_at.is_some() && r.next_reminder_at.is_none());
    assert_eq!(env.run_sweep(at(19, 10, 0)).await.executed, 0);
}

#[rstest]
#[tokio::test]
async fn tenant_disable_emails_and_kind_toggle_hold_without_burning(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;

    env.set_tenant_emails(false).await;
    assert_eq!(env.run_sweep(at(11, 10, 0)).await.executed, 0);
    assert_eq!(env.run_sweep(at(18, 10, 0)).await.executed, 0);
    env.set_tenant_emails(true).await;
    env.set_reminder_kind(INVOICING_ENTITY_ID, false).await;
    assert_eq!(env.run_sweep(at(18, 10, 0)).await.executed, 0);
    let r = row(&env, invoice).await;
    assert_eq!(
        (r.last_offset_days, r.next_reminder_at),
        (None, Some(fire(10)))
    );
    assert!(queued(&env).await.is_empty());

    env.set_reminder_kind(INVOICING_ENTITY_ID, true).await;
    let stats = env.run_sweep(at(18, 10, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    assert_eq!(
        queued(&env).await,
        vec![(reminder_dedupe_key(invoice, 1, 7), 0)]
    );
    let acts = activities(&env, invoice).await;
    let executed = of_type(&acts, "payment_reminder.step_executed");
    assert_eq!(meta(executed[0], "skipped_offsets"), Some(&json!([0])));
}

#[rstest]
#[tokio::test]
async fn processing_or_live_payment_defers_without_advancing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.skip_credit_invariants(
        "marks an invoice paid without a payment, to exercise the reminder states",
    );
    default_with_steps(&env, &[0, 7]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;

    // A SEPA debit is pending for three days across the day-0 step.
    set_status(&env, invoice, InvoicePaymentStatus::Processing).await;
    for day in 10..13 {
        let stats = env.run_sweep(at(day, 10, 0)).await;
        assert_eq!(stats.executed, 0, "{stats:?}");
    }
    let r = row(&env, invoice).await;
    assert_eq!(r.last_offset_days, None);
    assert_eq!(r.state, ReminderStateEnum::Enrolled);
    assert!(queued(&env).await.is_empty());

    set_status(&env, invoice, InvoicePaymentStatus::Paid).await;
    let stats = env.run_sweep(at(13, 10, 0)).await;
    assert_eq!(stats.ended, 1, "{stats:?}");
    let r = row(&env, invoice).await;
    assert_eq!(r.state, ReminderStateEnum::Ended);
    assert!(r.next_reminder_at.is_none() && r.ended_at.is_some());
}

// ---- lifecycle hooks -----------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn paid_mid_sequence_clears_schedule(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    assert_eq!(row(&env, invoice).await.last_offset_days, Some(0));

    env.services()
        .mark_invoice_as_paid(
            Actor::System,
            TENANT_ID,
            invoice,
            rust_decimal::Decimal::from(100),
            Utc::now().fixed_offset(),
            None,
        )
        .await
        .expect("mark paid");
    env.run_outbox_and_orchestration().await;
    let r = row(&env, invoice).await;
    assert_eq!(r.state, ReminderStateEnum::Ended);
    assert!(r.next_reminder_at.is_none() && r.ended_at.is_some());
    assert_eq!(r.last_offset_days, Some(0), "cursor kept for the UI");
    assert_eq!(env.run_sweep(at(17, 10, 0)).await.executed, 0);
}

#[rstest]
#[tokio::test]
async fn voided_and_uncollectible_stop(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;

    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    let voided = env.get_invoices(sub_id).await[0].id;
    let uncollectible = env
        .finalized_invoice_with_pdf(FinalizedInvoice::one_off(CUST_SPOTIFY_ID))
        .await;
    env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!(row(&env, voided).await.state, ReminderStateEnum::Enrolled);
    assert_eq!(
        row(&env, uncollectible).await.state,
        ReminderStateEnum::Enrolled
    );

    env.try_void_invoice(voided).await.expect("void");
    env.store()
        .mark_invoice_as_uncollectible(uncollectible, TENANT_ID)
        .await
        .expect("uncollectible");
    for invoice in [voided, uncollectible] {
        let r = row(&env, invoice).await;
        assert_eq!(r.state, ReminderStateEnum::Ended, "{invoice}");
        assert!(r.next_reminder_at.is_none() && r.ended_at.is_some());
    }
}

#[rstest]
#[tokio::test]
async fn late_failure_reopen_restarts_and_sends_one_step(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    default_with_steps(&env, &[-3, 0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let today = Utc::now().date_naive();
    let day = |d: i64, h: u32| {
        (today + Duration::days(d))
            .and_hms_opt(h, 0, 0)
            .expect("time")
    };
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(day(10, 0), day(-1, 8)),
        )
        .await;
    env.run_sweep(day(0, 10)).await;
    let first = row(&env, invoice).await;
    assert_eq!(
        (first.generation, first.next_reminder_at),
        (1, Some(day(7, 9)))
    );

    // Charged and settled, so the ladder ends.
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(TENANT_ID, invoice, CUST_UBER_PAYMENT_METHOD_ID, None)
        .await
        .expect("charge");
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"pi_reopen","amount":10000,"currency":"EUR"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    assert_eq!(
        env.get_customer_invoices(CUST_UBER_ID).await[0].payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Paid
    );
    assert_eq!(row(&env, invoice).await.state, ReminderStateEnum::Ended);
    env.drain_email_queue().await;

    // The bank claws the money back: the invoice reopens and the ladder restarts as the system.
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_rev_{tx}","kind":"payment_late_failure","transaction_id":"{tx}","external_id":"pi_reopen"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    let r = row(&env, invoice).await;
    assert_eq!(r.state, ReminderStateEnum::Enrolled);
    assert_eq!(r.generation, 2);
    assert!(r.ended_at.is_none());
    assert_eq!(r.next_reminder_at, Some(day(7, 9)));
    let acts = activities(&env, invoice).await;
    assert_eq!(of_type(&acts, "payment_reminder.restarted").len(), 1);

    let stats = env.run_sweep(day(8, 10)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert_eq!(queued(&env).await.len(), 0);
}

#[rstest]
#[tokio::test]
async fn reopen_of_an_in_flight_ladder_never_resends_a_step(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    default_with_steps(&env, &[-3, 0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let today = Utc::now().date_naive();
    let day = |d: i64, h: u32| {
        (today + Duration::days(d))
            .and_hms_opt(h, 0, 0)
            .expect("time")
    };
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(day(10, 0), day(-1, 8)),
        )
        .await;
    env.run_sweep(day(0, 10)).await;
    assert_eq!(row(&env, invoice).await.generation, 1);

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(TENANT_ID, invoice, CUST_UBER_PAYMENT_METHOD_ID, None)
        .await
        .expect("charge");
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"pi_inflight","amount":10000,"currency":"EUR"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    assert_eq!(row(&env, invoice).await.state, ReminderStateEnum::Ended);
    env.drain_email_queue().await;

    // The bank claws the payment back: the ended ladder restarts as the system.
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_lf_{tx}","kind":"payment_late_failure","transaction_id":"{tx}","external_id":"pi_inflight"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    assert_eq!(
        env.get_customer_invoices(CUST_UBER_ID).await[0].payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Unpaid
    );
    let r = row(&env, invoice).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 2));
    assert_eq!(r.next_reminder_at, Some(day(7, 9)));

    // The first step goes out.
    assert_eq!(env.run_sweep(day(8, 10)).await.executed, 1);
    assert_eq!(env.drain_email_queue().await.len(), 1);
    let r = row(&env, invoice).await;
    assert_eq!(
        (r.last_offset_days, r.next_reminder_at),
        (Some(-3), Some(day(10, 9)))
    );

    // A cancelled system charge hands the invoice back to reminders again. The ladder is in
    // flight, so the row is left alone and the step already received is never queued again.
    env.set_mock_charge_behavior(MockChargeBehavior::Cancelled)
        .await;
    env.services()
        .complete_invoice_payment_automatic(TENANT_ID, invoice, CUST_UBER_PAYMENT_METHOD_ID, None)
        .await
        .expect("cancelled charge");
    env.run_outbox_and_orchestration().await;
    let r = row(&env, invoice).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 2));
    assert_eq!(
        (r.last_offset_days, r.next_reminder_at),
        (Some(-3), Some(day(10, 9)))
    );
    let acts = activities(&env, invoice).await;
    assert_eq!(of_type(&acts, "payment_reminder.restarted").len(), 1);
    assert_eq!(env.run_sweep(day(8, 11)).await.executed, 0);
    assert!(env.drain_email_queue().await.is_empty());
    let reminders_sent = sent_rows(&env)
        .await
        .iter()
        .filter(|r| r.kind.as_deref() == Some("payment_reminder"))
        .count();
    assert_eq!(reminders_sent, 1);
}

#[rstest]
#[tokio::test]
async fn a_dispute_keeps_the_invoice_paid_and_its_reminders_ended(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    default_with_steps(&env, &[-3, 0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let today = Utc::now().date_naive();
    let day = |d: i64, h: u32| {
        (today + Duration::days(d))
            .and_hms_opt(h, 0, 0)
            .expect("time")
    };
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(day(10, 0), day(-1, 8)),
        )
        .await;
    env.run_sweep(day(0, 10)).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(TENANT_ID, invoice, CUST_UBER_PAYMENT_METHOD_ID, None)
        .await
        .expect("charge");
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"pi_disputed","amount":10000,"currency":"EUR"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    assert_eq!(row(&env, invoice).await.state, ReminderStateEnum::Ended);
    env.drain_email_queue().await;
    // Dispute webhooks are matched by the provider's id of the settled transaction.
    let ext = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice)
        .await
        .expect("transactions")
        .into_iter()
        .find_map(|t| t.transaction.provider_transaction_id)
        .expect("settled transaction has a provider id");

    // The payer contests 4000: lost to the dispute, the invoice stays paid, nothing to chase.
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_dw_1","kind":"dispute_funds_withdrawn","external_id":"{ext}","amount":4000,"currency":"EUR","occurred_at":"{at}"}}"#,
            at = Utc::now().to_rfc3339(),
        ),
    )
    .await;
    assert_eq!(
        env.get_customer_invoices(CUST_UBER_ID).await[0].payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Paid
    );
    let r = row(&env, invoice).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Ended, 1));
    assert_eq!(r.stop_reason, None);
    let acts = activities(&env, invoice).await;
    assert!(of_type(&acts, "payment_reminder.restarted").is_empty());
    assert!(of_type(&acts, "payment_reminder.stopped").is_empty());
    assert_eq!(env.run_sweep(day(8, 10)).await.executed, 0);
    assert!(env.drain_email_queue().await.is_empty());
}

#[rstest]
#[tokio::test]
async fn operator_stop_survives_a_reopen(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    default_with_steps(&env, &[-3, 0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let today = Utc::now().date_naive();
    let day = |d: i64, h: u32| {
        (today + Duration::days(d))
            .and_hms_opt(h, 0, 0)
            .expect("time")
    };
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(day(10, 0), day(-1, 8)),
        )
        .await;
    env.run_sweep(day(0, 10)).await;
    env.store()
        .stop_invoice_reminders(ACTOR, TENANT_ID, invoice, day(0, 11))
        .await
        .expect("stop");

    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(TENANT_ID, invoice, CUST_UBER_PAYMENT_METHOD_ID, None)
        .await
        .expect("charge");
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"pi_stopped","amount":10000,"currency":"EUR"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    let r = row(&env, invoice).await;
    assert_eq!(r.state, ReminderStateEnum::Ended);
    assert!(r.stopped_at.is_some());
    env.drain_email_queue().await;

    // The late failure reopens the invoice, but a system hook never clears an operator stop.
    deliver_webhook(
        &env,
        format!(
            r#"{{"id":"evt_rev_{tx}","kind":"payment_late_failure","transaction_id":"{tx}","external_id":"pi_stopped"}}"#,
            tx = tx.id.as_base62(),
        ),
    )
    .await;
    assert_eq!(
        env.get_customer_invoices(CUST_UBER_ID).await[0].payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Unpaid
    );
    let r = row(&env, invoice).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 1));
    assert!(r.ended_at.is_none());
    assert!(r.stopped_at.is_some());
    assert_eq!(r.stop_reason, Some(ReminderStopReasonEnum::Operator));
    assert_eq!(r.next_reminder_at, None);
    let acts = activities(&env, invoice).await;
    assert!(of_type(&acts, "payment_reminder.restarted").is_empty());
    assert_eq!(env.run_sweep(day(8, 10)).await.executed, 0, "still stopped");
    assert!(env.drain_email_queue().await.is_empty());

    // Only the operator restarts it.
    let state = env
        .store()
        .restart_invoice_reminders(ACTOR, TENANT_ID, invoice, Utc::now().naive_utc())
        .await
        .expect("restart");
    assert_eq!((state.generation, state.stopped_at), (2, None));
    assert_eq!(state.next_reminder_at, Some(day(7, 9)));
}

// ---- automatic collection handed over to reminders -------------------------------------------

/// A subscription invoice collected through the real pipeline: `request_auto_charge` stamps it
/// and the queued charge runs with `behavior`. Returns the invoice as the pipeline left it.
async fn auto_charged_invoice(
    env: &TestEnv,
    behavior: MockChargeBehavior,
) -> meteroid_store::domain::Invoice {
    env.seed_payments().await;
    env.set_mock_charge_behavior(behavior).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .on_start()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id).await[0].clone()
}

async fn load_invoice(env: &TestEnv, invoice: InvoiceId) -> meteroid_store::domain::Invoice {
    env.store()
        .get_invoice_by_id(TENANT_ID, invoice)
        .await
        .expect("invoice")
}

/// Claws back the invoice's settled charge with a full-amount reversal of mock `kind`.
async fn claw_back(env: &TestEnv, invoice: InvoiceId, kind: &str) {
    let tx = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice)
        .await
        .expect("transactions")
        .into_iter()
        .map(|t| t.transaction)
        .find(|t| t.status == PaymentStatusEnum::Settled)
        .expect("a settled charge");
    deliver_webhook(
        env,
        format!(
            r#"{{"id":"evt_cb_{tx}","kind":"{kind}","transaction_id":"{tx}","external_id":"{ext}"}}"#,
            tx = tx.id.as_base62(),
            ext = tx.provider_transaction_id.clone().unwrap_or_default(),
        ),
    )
    .await;
}

/// The bank reclaims the charge (GoCardless `late_failure_settled`, insufficient funds).
async fn late_failure(env: &TestEnv, invoice: InvoiceId) {
    claw_back(env, invoice, "payment_late_failure").await;
}

/// The payer contests the charge (GoCardless `charged_back`): the invoice stays paid.
async fn charge_back(env: &TestEnv, invoice: InvoiceId) {
    claw_back(env, invoice, "payment_charged_back").await;
}

#[rstest]
#[tokio::test]
async fn late_failure_on_auto_charged_invoice_hands_it_to_reminders(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = auto_charged_invoice(&env, MockChargeBehavior::Succeeded).await;
    assert_eq!(
        invoice.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Paid
    );
    assert!(
        invoice.auto_charge_requested_at.is_some(),
        "stamped by request_auto_charge"
    );
    assert!(env.reminder_row(invoice.id).await.is_none());

    // The hook enrols it itself, so a late claw-back is not bound by the sweep's look-back.
    late_failure(&env, invoice.id).await;
    let reopened = load_invoice(&env, invoice.id).await;
    assert_eq!(
        reopened.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Unpaid
    );
    assert!(
        reopened.auto_charge_requested_at.is_none(),
        "automatic collection is over"
    );
    let r = row(&env, invoice.id).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 1));
    assert!(r.next_reminder_at.is_some());
    let acts = activities(&env, invoice.id).await;
    assert_eq!(of_type(&acts, "payment_reminder.restarted").len(), 1);

    // The sweep finds the row and never enrols it a second time.
    let stats = env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!((stats.enrolled, stats.not_enrolled), (0, 0), "{stats:?}");
}

#[rstest]
#[tokio::test]
async fn late_failure_restarts_the_ended_ladder_of_an_errored_auto_charge(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = auto_charged_invoice(&env, MockChargeBehavior::Failed).await;
    assert_eq!(
        invoice.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Errored
    );
    assert!(invoice.auto_charge_requested_at.is_some());
    env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!(
        row(&env, invoice.id).await.state,
        ReminderStateEnum::Enrolled
    );

    // A later automatic charge collects it, which ends the ladder.
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(
            TENANT_ID,
            invoice.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            None,
        )
        .await
        .expect("charge");
    assert_eq!(tx.status, PaymentStatusEnum::Settled);
    env.run_outbox_and_orchestration().await;
    let paid = load_invoice(&env, invoice.id).await;
    assert_eq!(
        paid.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Paid
    );
    assert!(paid.auto_charge_requested_at.is_some());
    assert_eq!(row(&env, invoice.id).await.state, ReminderStateEnum::Ended);

    late_failure(&env, invoice.id).await;
    assert!(
        load_invoice(&env, invoice.id)
            .await
            .auto_charge_requested_at
            .is_none()
    );
    let r = row(&env, invoice.id).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 2));
    assert!(r.ended_at.is_none() && r.next_reminder_at.is_some());
}

#[rstest]
#[tokio::test]
async fn chargeback_on_auto_charged_invoice_keeps_it_paid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = auto_charged_invoice(&env, MockChargeBehavior::Succeeded).await;

    charge_back(&env, invoice.id).await;
    let disputed = load_invoice(&env, invoice.id).await;
    assert_eq!(
        (disputed.payment_status, disputed.amount_due),
        (meteroid_store::domain::enums::InvoicePaymentStatus::Paid, 0)
    );
    assert!(disputed.payment_disputed_at.is_some());
    assert!(
        env.reminder_row(invoice.id).await.is_none(),
        "nothing to chase"
    );
    let stats = env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!((stats.enrolled, stats.executed), (0, 0), "{stats:?}");
}

/// A failed collection with no cause the bank explained reopens the invoice but leaves the
/// decision to chase it to an operator.
#[rstest]
#[tokio::test]
async fn unexplained_reversal_on_auto_charged_invoice_holds_it_for_the_operator(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = auto_charged_invoice(&env, MockChargeBehavior::Succeeded).await;
    assert!(env.reminder_row(invoice.id).await.is_none());

    claw_back(&env, invoice.id, "payment_reversed").await;
    let reopened = load_invoice(&env, invoice.id).await;
    assert_eq!(
        reopened.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Unpaid
    );
    assert!(
        reopened.auto_charge_requested_at.is_none(),
        "automatic collection is over"
    );
    // A bare stopped row, so the enrolment scan never picks the invoice up either.
    let r = row(&env, invoice.id).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 0));
    assert_eq!(r.stop_reason, Some(ReminderStopReasonEnum::PaymentDisputed));
    assert!(r.sequence_id.is_none() && r.next_reminder_at.is_none());
    let stats = env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!((stats.enrolled, stats.executed), (0, 0), "{stats:?}");

    // The operator's restart enrols it.
    let state = env
        .store()
        .restart_invoice_reminders(ACTOR, TENANT_ID, invoice.id, Utc::now().naive_utc())
        .await
        .expect("restart");
    assert_eq!((state.generation, state.stopped_at), (1, None));
    assert!(row(&env, invoice.id).await.sequence_id.is_some());
}

#[rstest]
#[tokio::test]
async fn cancelled_auto_charge_hands_the_invoice_to_reminders(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = auto_charged_invoice(&env, MockChargeBehavior::Cancelled).await;
    let tx = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("transactions");
    assert_eq!(tx.len(), 1);
    assert_eq!(tx[0].transaction.status, PaymentStatusEnum::Cancelled);
    assert_eq!(
        invoice.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Unpaid
    );
    assert!(
        invoice.auto_charge_requested_at.is_none(),
        "a cancelled system charge is never retried"
    );
    let r = row(&env, invoice.id).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 1));
    assert!(r.next_reminder_at.is_some());

    let stats = env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!((stats.enrolled, stats.not_enrolled), (0, 0), "{stats:?}");
}

#[rstest]
#[tokio::test]
async fn late_invoice_paid_never_ends_a_restarted_ladder(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;
    env.run_outbox_and_orchestration().await;
    let invoice = env.get_invoices(sub_id).await[0].id;
    env.run_sweep(Utc::now().naive_utc()).await;
    assert_eq!(row(&env, invoice).await.state, ReminderStateEnum::Enrolled);

    // A debt-cancellation note settles the invoice; its paid event ends the ladder.
    let note = env
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice,
                line_items: vec![],
                reason: Some("Written off".to_string()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("debt cancellation");
    let paid = load_invoice(&env, invoice).await;
    assert_eq!(
        paid.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Paid
    );
    env.run_outbox_and_orchestration().await;
    assert_eq!(row(&env, invoice).await.state, ReminderStateEnum::Ended);

    // Voiding the note reopens the invoice: the ladder restarts.
    env.store()
        .void_credit_note(Actor::System, TENANT_ID, note.id)
        .await
        .expect("void credit note");
    let r = row(&env, invoice).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 2));

    // The paid event is redelivered late: the invoice is payable again, so nothing ends.
    env.services()
        .on_invoice_paid((&paid).into(), TENANT_ID)
        .await
        .expect("invoice_paid replay");
    let r = row(&env, invoice).await;
    assert_eq!((r.state, r.generation), (ReminderStateEnum::Enrolled, 2));
    assert!(r.ended_at.is_none() && r.next_reminder_at.is_some());
}

#[rstest]
#[tokio::test]
async fn reversal_commits_when_the_reminder_restart_fails(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let corrupt = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Corrupt",
            customer_audience(CUST_UBER_ID),
            &[0],
            true,
        )
        .await;
    let invoice = auto_charged_invoice(&env, MockChargeBehavior::Succeeded).await;
    assert_eq!(
        invoice.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Paid
    );
    // The stored audience no longer deserializes: resolving any sequence of the entity fails.
    sql(
        &env,
        &format!(
            "UPDATE payment_reminder_sequence SET audience = '{{\"predicates\": \"corrupt\"}}'::jsonb \
             WHERE id = '{}'",
            corrupt.id.as_uuid()
        ),
    )
    .await;

    // `deliver_webhook` panics if the dispatcher fails: the reversal must commit regardless.
    late_failure(&env, invoice.id).await;
    let reopened = load_invoice(&env, invoice.id).await;
    assert_eq!(
        reopened.payment_status,
        meteroid_store::domain::enums::InvoicePaymentStatus::Unpaid
    );
    assert!(reopened.auto_charge_requested_at.is_none());
    let tx = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("transactions");
    assert_eq!(tx[0].transaction.status, PaymentStatusEnum::Refunded);
    assert!(
        env.reminder_row(invoice.id).await.is_none(),
        "the failed restart rolled back to its savepoint"
    );
}

// ---- generation, worker checks -------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn restart_after_stop_sends_again_thanks_to_generation(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    assert_eq!(env.drain_email_queue().await.len(), 1);

    env.store()
        .stop_invoice_reminders(ACTOR, TENANT_ID, invoice, at(10, 11, 0))
        .await
        .expect("stop");
    assert_eq!(env.run_sweep(at(17, 10, 0)).await.executed, 0, "stopped");

    let state = env
        .store()
        .restart_invoice_reminders(ACTOR, TENANT_ID, invoice, at(10, 12, 0))
        .await
        .expect("restart");
    assert_eq!(state.generation, 2);
    assert_eq!(state.stopped_at, None);
    assert_eq!(
        (state.last_offset_days, state.next_reminder_at),
        (None, Some(fire(10)))
    );

    let stats = env.run_sweep(at(10, 12, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    assert_eq!(
        queued(&env).await,
        vec![(reminder_dedupe_key(invoice, 2, 0), 0)]
    );
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1, "a fresh dedupe key sends again");
    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].dedupe_key, rows[1].dedupe_key);
}

#[rstest]
#[tokio::test]
async fn superseded_message_is_dropped_with_activity(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    assert_eq!(queued(&env).await.len(), 1);

    env.store()
        .restart_invoice_reminders(ACTOR, TENANT_ID, invoice, at(10, 11, 0))
        .await
        .expect("restart");
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, invoice).await;
    let skipped = of_type(&acts, "payment_reminder.skipped");
    assert_eq!(skipped.len(), 1);
    assert_eq!(meta(skipped[0], "reason"), Some(&json!("superseded")));
    assert!(sent_rows(&env).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn send_now_advances_cursor_for_future_step_and_refuses_paid_invoice(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.skip_credit_invariants(
        "marks an invoice paid without a payment, to exercise the reminder states",
    );
    default_with_steps(&env, &[0, 7, 14]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;

    env.store()
        .send_invoice_reminder_now(ACTOR, TENANT_ID, invoice, 7, "click-1", at(2, 10, 0))
        .await
        .expect("send now");
    let r = row(&env, invoice).await;
    assert_eq!(r.last_offset_days, Some(7));
    assert_eq!(r.next_reminder_at, Some(fire(24)));
    // The same click again is collapsed by the request id once the claim exists.
    assert_eq!(
        queued(&env).await,
        vec![(manual_reminder_dedupe_key(invoice, "click-1"), 0)]
    );
    assert_eq!(env.drain_email_queue().await.len(), 1);
    env.store()
        .send_invoice_reminder_now(ACTOR, TENANT_ID, invoice, 7, "click-1", at(2, 10, 1))
        .await
        .expect("idempotent");
    assert!(env.drain_email_queue().await.is_empty());

    let not_a_step = env
        .store()
        .send_invoice_reminder_now(ACTOR, TENANT_ID, invoice, 3, "click-2", at(2, 10, 2))
        .await
        .expect_err("not a step");
    assert!(matches!(
        not_a_step.current_context(),
        StoreError::InvalidArgument(_)
    ));

    set_status(&env, invoice, InvoicePaymentStatus::Paid).await;
    assert_failed_precondition(
        env.store()
            .send_invoice_reminder_now(ACTOR, TENANT_ID, invoice, 14, "click-3", at(2, 10, 3))
            .await,
        "not open for payment",
    );
}

#[rstest]
#[tokio::test]
async fn send_now_requires_a_sequence_row(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7, 14]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let fresh = open_invoice(&env, CUST_UBER_ID, 10).await;
    let stale = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(due(-15), at(-20, 8, 0)),
        )
        .await;
    assert_failed_precondition(
        env.store()
            .send_invoice_reminder_now(ACTOR, TENANT_ID, fresh, 0, "r1", at(0, 10, 0))
            .await,
        "Restart reminders first",
    );
    env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(row(&env, stale).await.state, ReminderStateEnum::NotEnrolled);
    assert_failed_precondition(
        env.store()
            .send_invoice_reminder_now(ACTOR, TENANT_ID, stale, 0, "r2", at(0, 10, 0))
            .await,
        "Restart reminders first",
    );

    let state = env
        .store()
        .restart_invoice_reminders(ACTOR, TENANT_ID, stale, at(0, 10, 0))
        .await
        .expect("restart a stale ladder");
    assert_eq!(
        state.last_offset_days,
        Some(7),
        "the last elapsed step (+14) executes next"
    );
    assert_eq!(state.next_reminder_at, Some(fire(-1)));
    env.store()
        .send_invoice_reminder_now(ACTOR, TENANT_ID, stale, 0, "r3", at(0, 10, 1))
        .await
        .expect("send now after restart");
    assert_eq!(env.drain_email_queue().await.len(), 1);

    // A restart racing the click: the message carries the old generation and is dropped.
    env.store()
        .send_invoice_reminder_now(ACTOR, TENANT_ID, stale, 7, "r4", at(0, 10, 2))
        .await
        .expect("send now");
    env.store()
        .restart_invoice_reminders(ACTOR, TENANT_ID, stale, at(0, 10, 3))
        .await
        .expect("restart");
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, stale).await;
    let skipped = of_type(&acts, "payment_reminder.skipped");
    assert_eq!(skipped.len(), 1);
    assert_eq!(meta(skipped[0], "reason"), Some(&json!("superseded")));
}

#[rstest]
#[tokio::test]
async fn zero_step_sequence_sends_nothing_to_its_audience(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[0, 7]).await;
    let silent = env
        .create_sequence_with_audience(
            INVOICING_ENTITY_ID,
            "Silent",
            customer_audience(CUST_UBER_ID),
            &[],
            true,
        )
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats.enrolled, 1, "{stats:?}");
    let r = row(&env, invoice).await;
    assert_eq!(r.sequence_id, Some(silent.id));
    assert_ne!(r.sequence_id, Some(default.id));
    assert_eq!(r.next_reminder_at, None);
    assert!(r.completed_at.is_some());
    for day in [10, 17, 30] {
        assert_eq!(env.run_sweep(at(day, 10, 0)).await.executed, 0);
    }
    assert!(queued(&env).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn disabled_default_leaves_invoice_unresolved_until_enabled(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.enable_reminders().await;
    env.default_sequence(INVOICING_ENTITY_ID, false).await;
    env.create_sequence_with_audience(
        INVOICING_ENTITY_ID,
        "Comodo",
        customer_audience(CUST_COMODO_ID),
        &[0],
        true,
    )
    .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!((stats.enrolled, stats.not_enrolled), (0, 0), "{stats:?}");
    assert!(env.reminder_row(invoice).await.is_none());
    assert!(reminder_activities(&env, invoice).await.is_empty());
    let state = env
        .store()
        .get_invoice_reminder_state(TENANT_ID, invoice, at(0, 10, 0))
        .await
        .expect("state");
    assert_eq!(
        state.not_eligible_reason,
        Some(NotEligibleReason::NoEnabledSequence)
    );
    assert!(state.would_resolve_to.is_none());

    let default = env.default_sequence(INVOICING_ENTITY_ID, true).await;
    let stats = env.run_sweep(at(1, 10, 0)).await;
    assert_eq!(stats.enrolled, 1, "{stats:?}");
    assert_eq!(row(&env, invoice).await.sequence_id, Some(default.id));
}

#[rstest]
#[tokio::test]
async fn quarantine_defers_without_advancing_cursor(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;

    // The enqueue inside the claim transaction fails: the transaction rolls back.
    sql(
        &env,
        "ALTER TABLE pgmq.q_send_email_request RENAME TO q_send_email_request_off",
    )
    .await;
    let stats = env.run_sweep(at(10, 10, 0)).await;
    sql(
        &env,
        "ALTER TABLE pgmq.q_send_email_request_off RENAME TO q_send_email_request",
    )
    .await;
    assert_eq!((stats.executed, stats.quarantined), (0, 1), "{stats:?}");
    let r = row(&env, invoice).await;
    assert_eq!(
        r.last_offset_days, None,
        "the cursor is never nulled or moved"
    );
    assert_eq!(
        r.next_reminder_at,
        Some(at(10, 11, 0)),
        "looked at again in an hour"
    );
    assert!(queued(&env).await.is_empty());
    let acts = activities(&env, invoice).await;
    assert!(of_type(&acts, "payment_reminder.step_executed").is_empty());

    let stats = env.run_sweep(at(10, 11, 0)).await;
    assert_eq!(stats.executed, 1, "{stats:?}");
    assert_eq!(row(&env, invoice).await.last_offset_days, Some(0));
}

#[rstest]
#[tokio::test]
async fn ladder_stale_is_permanent(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = default_with_steps(&env, &[-3, 0, 7, 14]).await;
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(due(-15), at(-20, 8, 0)),
        )
        .await;
    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(stats.not_enrolled, 1, "{stats:?}");
    let r = row(&env, invoice).await;
    assert_eq!(r.state, ReminderStateEnum::NotEnrolled);
    assert_eq!(
        r.not_enrolled_reason,
        Some(ReminderNotEnrolledReasonEnum::LadderStale)
    );
    let acts = activities(&env, invoice).await;
    let not_enrolled = of_type(&acts, "payment_reminder.not_enrolled");
    assert_eq!(not_enrolled.len(), 1);
    assert_eq!(
        meta(not_enrolled[0], "reason"),
        Some(&json!("LADDER_STALE"))
    );

    env.set_sequence_steps(default.id, &[-3, 0, 7, 14, 90])
        .await;
    let stats = env.run_sweep(at(1, 10, 0)).await;
    assert_eq!((stats.enrolled, stats.not_enrolled), (0, 0), "{stats:?}");
    assert_eq!(
        row(&env, invoice).await.state,
        ReminderStateEnum::NotEnrolled
    );
    let state = env
        .store()
        .get_invoice_reminder_state(TENANT_ID, invoice, at(1, 10, 0))
        .await
        .expect("state");
    assert_eq!(
        state.not_eligible_reason,
        Some(NotEligibleReason::LadderStale)
    );
}

#[rstest]
#[tokio::test]
async fn candidate_scan_is_bounded_and_keyset_survives_a_backlog(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0]).await;
    let template = open_invoice(&env, CUST_UBER_ID, 5).await;
    let clone = |n: i64, tag: &str, due_at: NaiveDateTime, finalized_at: NaiveDateTime| {
        format!(
            "INSERT INTO invoice SELECT (json_populate_record(i, json_build_object( \
               'id', gen_random_uuid(), 'invoice_number', 'BL-{tag}-' || g, \
               'due_at', '{due_at}', 'finalized_at', '{finalized_at}'))).* \
             FROM invoice i, generate_series(1, {n}) g WHERE i.id = '{}'",
            template.as_uuid()
        )
    };
    // Far beyond the ladder's reach: never scanned.
    sql(&env, &clone(6_000, "old", due(-60), at(-60, 8, 0))).await;
    // Inside the reach but stale (fired 25 h ago, before finalization): stamped, never enrolled.
    sql(&env, &clone(700, "stale", due(-2), at(-1, 20, 0))).await;

    let stats = env.run_sweep(at(0, 10, 0)).await;
    assert_eq!((stats.enrolled, stats.not_enrolled), (1, 700), "{stats:?}");
    assert!(!stats.truncated);
    const { assert!(700 > ENROLL_BATCH, "two pages, so the keyset was used") };
    let r = row(&env, template).await;
    assert_eq!(r.state, ReminderStateEnum::Enrolled);
    assert_eq!(r.next_reminder_at, Some(fire(5)));
    assert_eq!(
        env.run_sweep(at(0, 11, 0)).await.not_enrolled,
        0,
        "stamps are permanent"
    );
}

// ---- send window ---------------------------------------------------------------------------

#[rstest]
#[tokio::test]
async fn send_window_fires_at_entity_local_hour_and_dst(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0]).await;
    env.store()
        .update_email_settings(
            ACTOR,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            EmailSettingsPatch {
                timezone: Some(Some("Europe/Paris".to_string())),
                ..Default::default()
            },
        )
        .await
        .expect("settings");
    let d = |m: u32, day: u32, h: u32, min: u32| {
        NaiveDate::from_ymd_opt(2026, m, day)
            .expect("date")
            .and_hms_opt(h, min, 0)
            .expect("time")
    };
    let winter = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(d(3, 28, 0, 0), d(3, 20, 8, 0)),
        )
        .await;
    let summer = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_SPOTIFY_ID).dates(d(3, 30, 0, 0), d(3, 20, 8, 0)),
        )
        .await;
    env.run_sweep(d(3, 25, 10, 0)).await;
    assert_eq!(
        row(&env, winter).await.next_reminder_at,
        Some(d(3, 28, 8, 0)),
        "09:00 CET"
    );
    assert_eq!(
        row(&env, summer).await.next_reminder_at,
        Some(d(3, 30, 7, 0)),
        "09:00 CEST"
    );

    assert_eq!(env.run_sweep(d(3, 28, 7, 59)).await.executed, 0);
    assert_eq!(env.run_sweep(d(3, 28, 8, 1)).await.executed, 1);
    assert_eq!(env.run_sweep(d(3, 30, 6, 59)).await.executed, 0);
    assert_eq!(env.run_sweep(d(3, 30, 7, 1)).await.executed, 1);
    let state = env
        .store()
        .get_invoice_reminder_state(TENANT_ID, summer, d(3, 30, 8, 0))
        .await
        .expect("state");
    assert_eq!(state.send_window.to_string(), "09:00 Europe/Paris");
}

#[rstest]
#[tokio::test]
async fn timezone_change_restamps_next_reminder(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    let today = Utc::now().date_naive();
    let day = |d: i64, h: u32| {
        (today + Duration::days(d))
            .and_hms_opt(h, 0, 0)
            .expect("time")
    };
    let invoice = env
        .finalized_invoice_with_pdf(
            FinalizedInvoice::one_off(CUST_UBER_ID).dates(day(20, 0), day(-1, 8)),
        )
        .await;
    env.run_sweep(day(0, 10)).await;
    assert_eq!(row(&env, invoice).await.next_reminder_at, Some(day(20, 9)));

    env.store()
        .update_email_settings(
            ACTOR,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            EmailSettingsPatch {
                timezone: Some(Some("Asia/Tokyo".to_string())),
                ..Default::default()
            },
        )
        .await
        .expect("settings");
    assert_eq!(
        row(&env, invoice).await.next_reminder_at,
        Some(day(20, 0)),
        "09:00 JST"
    );
    env.store()
        .update_email_settings(
            ACTOR,
            TENANT_ID,
            INVOICING_ENTITY_ID,
            EmailSettingsPatch {
                reminder_send_hour: Some(14),
                ..Default::default()
            },
        )
        .await
        .expect("settings");
    assert_eq!(
        row(&env, invoice).await.next_reminder_at,
        Some(day(20, 5)),
        "14:00 JST"
    );
    assert_eq!(row(&env, invoice).await.last_offset_days, None);
}

// ---- worker --------------------------------------------------------------------------------

fn localized_steps(offsets: &[i32]) -> Vec<PaymentReminderStepInput> {
    offsets
        .iter()
        .map(|o| PaymentReminderStepInput {
            offset_days: *o,
            content: EmailCopyByLocale(
                [
                    (
                        ENTITY_LOCALE,
                        "Rappel : facture {{invoice_number}}",
                        "Bonjour {{customer_name}},\n\nMerci de régler {{amount_due}}.",
                    ),
                    (
                        "en-US",
                        "Reminder: invoice {{invoice_number}}",
                        "Hi {{customer_name}},\n\nPlease pay {{amount_due}}.",
                    ),
                ]
                .into_iter()
                .map(|(l, s, b)| {
                    (
                        l.to_string(),
                        EmailCopy {
                            subject: s.to_string(),
                            body: b.to_string(),
                        },
                    )
                })
                .collect(),
            ),
        })
        .collect()
}

async fn localized_sequence(env: &TestEnv, customer: CustomerId) -> PaymentReminderSequenceId {
    env.store()
        .create_reminder_sequence(
            ACTOR,
            TENANT_ID,
            PaymentReminderSequenceNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Localized".to_string(),
                enabled: true,
                audience: Some(customer_audience(customer)),
                steps: localized_steps(&[0, 7]),
            },
        )
        .await
        .expect("sequence")
        .sequence
        .id
}

#[rstest]
#[tokio::test]
async fn worker_sends_pdf_attached_localized_reminder_with_dedupe_key(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.enable_reminders().await;
    env.default_sequence(INVOICING_ENTITY_ID, true).await;
    let sequence_id = localized_sequence(&env, CUST_UBER_ID).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["fr-FR"])
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    let key = reminder_dedupe_key(invoice, 1, 0);
    assert_eq!(queued(&env).await, vec![(key.clone(), 0)]);

    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    let email: &Email = &emails[0];
    assert!(
        email.subject.starts_with("Rappel : facture REM-"),
        "{}",
        email.subject
    );
    assert!(email.body_html.contains("Merci de régler"));
    assert!(email.body_html.contains(r#"<html lang="fr""#));
    assert_eq!(email.attachments.len(), 1);
    assert!(email.attachments[0].filename.ends_with(".pdf"));
    assert_eq!(email.to[0].email, RECIPIENT);
    let rows = sent_rows(&env).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].dedupe_key.as_deref(), Some(key.as_str()));
    assert_eq!(rows[0].locale.as_deref(), Some("fr-FR"));
    assert_eq!(rows[0].kind.as_deref(), Some("payment_reminder"));
    let state = env
        .store()
        .get_invoice_reminder_state(TENANT_ID, invoice, at(10, 11, 0))
        .await
        .expect("state");
    assert_eq!(state.sequence.as_ref().map(|s| s.id), Some(sequence_id));
    let sent = state
        .steps
        .iter()
        .find(|s| s.offset_days == 0)
        .expect("step 0");
    assert_eq!(
        sent.status,
        meteroid_store::domain::payment_reminders::InvoiceReminderStepStatus::Sent
    );
    assert_eq!(sent.sent_email_id, Some(rows[0].id));
    assert_eq!(state.enrolled_via, Some(ReminderEnrolledVia::Audience));
}

/// Extra customer-facing locales of the seeded (French) entity, on top of its fr-FR default.
async fn set_customer_extra_locales(env: &TestEnv, locales: &[&str]) {
    env.store()
        .patch_invoicing_entity(
            ACTOR,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                customer_extra_locales: Some(locales.iter().map(|l| l.to_string()).collect()),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("customer extra locales");
}

#[rstest]
#[tokio::test]
async fn worker_sends_the_whole_reminder_in_the_default_when_copy_lacks_the_locale(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.enable_reminders().await;
    env.default_sequence(INVOICING_ENTITY_ID, true).await;
    // Copy in fr-FR and en-US only; enabling de-DE afterwards is allowed.
    localized_sequence(&env, CUST_UBER_ID).await;
    set_customer_extra_locales(&env, &["de-DE"]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["de-DE"])
        .await;
    open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;

    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);
    assert!(
        emails[0].subject.starts_with("Rappel : facture"),
        "default-locale copy: {}",
        emails[0].subject
    );
    assert!(
        emails[0].body_html.contains(r#"<html lang="fr""#),
        "system text in the same language as the copy"
    );
    assert!(emails[0].body_html.contains("veuillez ignorer cet email"));
    assert!(!emails[0].body_html.contains("gegenstandslos"));
    assert_eq!(sent_rows(&env).await[0].locale.as_deref(), Some("fr-FR"));
}

#[rstest]
#[tokio::test]
async fn sequence_save_needs_copy_for_every_enabled_locale(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sequence_id = localized_sequence(&env, CUST_UBER_ID).await;
    // Enabling a locale the saved copy lacks is not blocked; the next save of the steps is.
    set_customer_extra_locales(&env, &["de-DE", "es-ES"]).await;
    let message = |err: error_stack::Report<StoreError>| match err.current_context() {
        StoreError::InvalidArgument(msg) => msg.clone(),
        other => panic!("expected InvalidArgument, got {other:?}"),
    };

    let created = env
        .store()
        .create_reminder_sequence(
            ACTOR,
            TENANT_ID,
            PaymentReminderSequenceNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Missing German".to_string(),
                enabled: true,
                audience: Some(customer_audience(CUST_SPOTIFY_ID)),
                steps: localized_steps(&[0, 7]),
            },
        )
        .await
        .expect_err("copy lacks de-DE and es-ES");
    assert_eq!(message(created), "Step 0: copy missing for de-DE, es-ES");

    let mut steps = localized_steps(&[7]);
    steps[0].content.0.insert(
        "de-DE".to_string(),
        EmailCopy {
            subject: "Erinnerung: Rechnung {{invoice_number}}".to_string(),
            body: "Hallo {{customer_name}},\n\nbitte zahlen Sie {{amount_due}}.".to_string(),
        },
    );
    let patch = |steps: Vec<PaymentReminderStepInput>| PaymentReminderSequencePatch {
        id: sequence_id,
        steps: Some(steps),
        ..Default::default()
    };
    let updated = env
        .store()
        .update_reminder_sequence(
            ACTOR,
            TENANT_ID,
            patch(steps.clone()),
            Utc::now().naive_utc(),
        )
        .await
        .expect_err("copy lacks es-ES");
    assert_eq!(message(updated), "Step +7: copy missing for es-ES");

    set_customer_extra_locales(&env, &["de-DE"]).await;
    env.store()
        .update_reminder_sequence(ACTOR, TENANT_ID, patch(steps), Utc::now().naive_utc())
        .await
        .expect("every enabled locale has copy");
}

#[rstest]
#[tokio::test]
async fn worker_skips_unpayable_with_activity(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.skip_credit_invariants(
        "marks an invoice paid without a payment, to exercise the reminder states",
    );
    default_with_steps(&env, &[0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    assert_eq!(queued(&env).await.len(), 1);

    set_status(&env, invoice, InvoicePaymentStatus::Paid).await;
    assert!(env.drain_email_queue().await.is_empty());
    let acts = activities(&env, invoice).await;
    let skipped = of_type(&acts, "payment_reminder.skipped");
    assert_eq!(skipped.len(), 1);
    assert_eq!(meta(skipped[0], "reason"), Some(&json!("not_payable")));
    assert!(sent_rows(&env).await.is_empty());
}

#[rstest]
#[tokio::test]
async fn preview_and_worker_render_identical_html(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0, 7]).await;
    env.set_customer_emails(CUST_UBER_ID, &[RECIPIENT], &["en-US"])
        .await;
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    env.run_sweep(at(10, 10, 0)).await;
    let r = row(&env, invoice).await;

    let pay_link = |id: InvoiceId| Ok(format!("https://pay.example/{id}"));
    let preview = match env
        .services()
        .build_customer_email(
            TENANT_ID,
            EmailKind::PaymentReminder,
            &EmailContext::PaymentReminder {
                invoice_id: invoice,
                sequence_id: r.sequence_id.expect("sequence"),
                offset_days: 0,
                generation: r.generation,
                manual: false,
            },
            None,
            None,
            &pay_link,
            false,
            SendOrigin::Automatic,
        )
        .await
        .expect("build")
    {
        BuildOutcome::Render(input) => {
            render(&input.layout, input.subject, input.body).expect("render")
        }
        _ => panic!("expected a render"),
    };
    let emails = env.drain_email_queue().await;
    assert_eq!(emails.len(), 1);

    let links = lazy_regex::regex!(r#"href="[^"]*""#);
    let normalize = |html: &str| links.replace_all(html, "href=\"#\"").to_string();
    assert_eq!(preview.subject, emails[0].subject);
    assert_eq!(
        normalize(&preview.body_html),
        normalize(&emails[0].body_html)
    );
}

#[rstest]
#[tokio::test]
async fn scan_sql_binds_now_and_never_uses_session_clock(#[future] test_env: TestEnv) {
    let env = test_env.await;
    default_with_steps(&env, &[0]).await;
    // Everything here is months in the past for the session clock; only the bound `now` counts.
    let invoice = open_invoice(&env, CUST_UBER_ID, 10).await;
    env.run_sweep(at(0, 10, 0)).await;
    assert_eq!(row(&env, invoice).await.next_reminder_at, Some(fire(10)));
    assert_eq!(
        env.run_sweep(at(9, 10, 0)).await.executed,
        0,
        "not due for the bound clock"
    );
    assert_eq!(env.run_sweep(at(10, 10, 0)).await.executed, 1);

    let state = env
        .store()
        .get_invoice_reminder_state(TENANT_ID, invoice, at(10, 11, 0))
        .await
        .expect("state");
    assert!(state.eligible(), "{:?}", state.not_eligible_reason);
    assert_eq!(state.last_offset_days, Some(0));
    assert!(state.completed_at.is_some());
}
