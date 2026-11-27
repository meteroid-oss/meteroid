//! The controller on facts: each run evaluates each live subscription due once and writes, in one
//! step, its transitions (status, period, plan), their logs and outbox stamped at the fact's
//! instant, and the documents due. Every test reads the stored state, the documents, the
//! subscription's events, MRR movements and outbox after each run.
//!
//! LeetCode is 35.00 a month in advance; Starter 29.00 + Seats 10.00, Pro 99.00 + 25.00; "Paid with
//! Free Trial" 49.00 after 14 free days; "Paid with Paid Trial" 99.00 with a 7-day paid trial.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, PlanId, PlanVersionId, PriceComponentId, PriceId, SubscriptionId,
};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use diesel_models::enums::{BillingPeriodEnum as DieselBillingPeriodEnum, SubscriptionStatusEnum};
use diesel_models::subscription_facts::SubscriptionEndingRow;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::credit::CreditGrantSource;
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface};
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{SubscriptionAssertExt, TestEnv, subscription, test_env, test_env_with_seed};
use crate::meteroid_it::container::SeedLevel;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

fn at(m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, m, d, h, 0, 0).unwrap()
}

/// Runs the controller over every due customer until none runs: what a run took made more due
/// at once (a renewal's draft past its grace, say).
async fn run(env: &TestEnv) {
    for _ in 0..10 {
        if env.run_billing_controller().await == 0 {
            return;
        }
    }
    panic!("the controller never settles");
}

/// Every invoice line and credit note line of the subscription, in order.
async fn documents(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    let mut invoices = env.get_invoices(sub_id).await;
    invoices.sort_by_key(|invoice| (invoice.billing_period_start, invoice.created_at));
    let mut documents = Vec::new();
    for invoice in invoices {
        let mut lines: Vec<String> = invoice
            .line_items
            .iter()
            .map(|line| {
                format!(
                    "{:?} {}: {} {}..{} {}",
                    invoice.invoice_type,
                    invoice.billing_period_start,
                    line.name,
                    line.start_date,
                    line.end_date,
                    line.amount_subtotal
                )
            })
            .collect();
        lines.sort();
        documents.extend(lines);
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            let mut lines: Vec<String> = note
                .line_items
                .iter()
                .map(|line| {
                    format!(
                        "note: {} {}..{} {}",
                        line.name, line.start_date, line.end_date, line.amount_subtotal
                    )
                })
                .collect();
            lines.sort();
            documents.extend(lines);
        }
    }
    documents
}

/// The subscription's invoices as (period start, status), in order.
async fn invoices(env: &TestEnv, sub_id: SubscriptionId) -> Vec<(NaiveDate, InvoiceStatusEnum)> {
    let mut invoices: Vec<_> = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .map(|invoice| (invoice.billing_period_start, invoice.status))
        .collect();
    invoices.sort_by_key(|(start, _)| *start);
    invoices
}

/// The subscription's MRR movements, as (type, change, day).
async fn movements(env: &TestEnv, sub_id: SubscriptionId) -> Vec<(String, i64, NaiveDate)> {
    use diesel_models::schema::bi_mrr_movement_log::dsl as m;
    m::bi_mrr_movement_log
        .filter(m::subscription_id.eq(sub_id))
        .order_by((m::applies_to.asc(), m::created_at.asc()))
        .select((m::movement_type, m::net_mrr_change, m::applies_to))
        .load::<(diesel_models::enums::MrrMovementType, i64, NaiveDate)>(&mut env.conn().await)
        .await
        .expect("movements")
        .into_iter()
        .map(|(kind, change, on)| (format!("{kind:?}"), change, on))
        .collect()
}

fn of_kind(movements: &[(String, i64, NaiveDate)], kind: &str) -> Vec<(String, i64, NaiveDate)> {
    movements
        .iter()
        .filter(|(k, _, _)| k == kind)
        .cloned()
        .collect()
}

/// The subscription's lifecycle events, as (type, day), in order.
async fn events(env: &TestEnv, sub_id: SubscriptionId) -> Vec<(String, NaiveDate)> {
    use diesel_models::schema::subscription_event::dsl as e;
    e::subscription_event
        .filter(e::subscription_id.eq(sub_id))
        .order_by((e::applies_to.asc(), e::created_at.asc()))
        .select((e::event_type, e::applies_to))
        .load::<(diesel_models::enums::SubscriptionEventType, NaiveDate)>(&mut env.conn().await)
        .await
        .expect("events")
        .into_iter()
        .map(|(kind, on)| (format!("{kind:?}"), on))
        .collect()
}

/// The subscription's lifecycle webhooks on the `webhook_out` queue: `updated <change>`,
/// `cancelled`, `ended`.
async fn outbox(env: &TestEnv, sub_id: SubscriptionId) -> Vec<String> {
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(500),
            MessageReadVtSec(0),
        )
        .await
        .expect("read webhook_out queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::SubscriptionUpdated(s) if s.subscription_id == sub_id => {
                Some(format!("updated {:?}", s.change_type))
            }
            OutboxEvent::SubscriptionCancelled(s) if s.subscription_id == sub_id => {
                Some("cancelled".to_owned())
            }
            OutboxEvent::SubscriptionEnded(s) if s.subscription_id == sub_id => {
                Some("ended".to_owned())
            }
            _ => None,
        })
        .collect()
}

fn count(outbox: &[String], event: &str) -> usize {
    outbox.iter().filter(|e| e.as_str() == event).count()
}

async fn leetcode_from_jan_1(env: &TestEnv) -> SubscriptionId {
    subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

/// An OnStart subscription starting later is Pending until its start, Active from it, its opening
/// invoice written by the run at its start and finalized after the entity's 12 hours of grace.
#[rstest]
#[tokio::test]
async fn a_subscription_is_activated_at_its_start(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 10, 12));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(2, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    run(&env).await;
    let pending = env.get_subscription(sub_id).await;
    pending
        .assert()
        .is_pending_activation()
        .has_pending_checkout(false);
    assert_eq!(
        pending.next_wake_at,
        Some(at(2, 1, 0)),
        "woken at its start"
    );
    assert_eq!(invoices(&env, sub_id).await, vec![]);

    env.set_now(at(2, 1, 1));
    run(&env).await;
    let active = env.get_subscription(sub_id).await;
    active
        .assert()
        .is_active()
        .has_pending_checkout(false)
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        vec!["Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500"]
    );
    assert_eq!(
        invoices(&env, sub_id).await,
        vec![(day(2, 1), InvoiceStatusEnum::Draft)],
        "in its grace"
    );
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "NewBusiness")
            .into_iter()
            .map(|(_, change, _)| change)
            .collect::<Vec<_>>(),
        vec![3500]
    );

    env.set_now(at(2, 1, 13));
    run(&env).await;
    assert_eq!(
        invoices(&env, sub_id).await,
        vec![(day(2, 1), InvoiceStatusEnum::Finalized)]
    );
    assert_eq!(
        env.get_subscription(sub_id).await.next_wake_at,
        Some(at(3, 1, 0))
    );
}

/// An OnCheckout subscription awaits its checkout from its start: Pending with pending_checkout,
/// and the controller writes no document of it even with a card on file to charge; the checkout
/// settles the start and writes its invoice, which no run writes again, and renewals follow.
#[rstest]
#[tokio::test]
async fn a_checkout_start_is_pending_until_it_settles(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 10));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    run(&env).await;
    let pending = env.get_subscription(sub_id).await;
    pending
        .assert()
        .is_pending_activation()
        .has_pending_checkout(true)
        .has_period_start(day(1, 1));
    assert_eq!(
        pending.current_period_end, None,
        "open-ended until it settles"
    );
    assert_eq!(
        invoices(&env, sub_id).await,
        vec![],
        "the checkout's to write"
    );

    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3500,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout");
    drop(conn);
    run(&env).await;
    let active = env.get_subscription(sub_id).await;
    active
        .assert()
        .is_active()
        .has_pending_checkout(false)
        .has_period_start(day(1, 1))
        .has_period_end(day(2, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        vec!["Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500"],
        "written once"
    );

    env.set_now(at(2, 1, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
        ]
    );
}

/// Decision 1: an OnCheckout free trial whose start settled during the trial (a checkout in it)
/// is Active at the trial's end, its first period billed then.
/// Rules: R-105
#[rstest]
#[tokio::test]
async fn a_free_trial_settled_during_it_is_active_at_its_end(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active()
        .has_pending_checkout(true)
        .has_period_start(day(1, 1))
        .has_period_end(day(1, 15));

    env.set_now(at(1, 5, 12));
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout during the trial");
    drop(conn);
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active()
        .has_pending_checkout(false);
    assert_eq!(
        invoices(&env, sub_id).await,
        vec![],
        "nothing billed in the trial"
    );

    env.set_now(at(1, 15, 1));
    run(&env).await;
    let active = env.get_subscription(sub_id).await;
    active
        .assert()
        .is_active()
        .has_pending_checkout(false)
        .has_period_start(day(1, 15));
    let opened = env.get_invoices(sub_id).await;
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].billing_period_start, day(1, 15));
    assert_eq!(opened[0].subtotal, 4900, "a whole month after the trial");
    assert_eq!(
        count(&outbox(&env, sub_id).await, "updated Some(TrialEnded)"),
        1
    );
}

/// Decision 1: a trial settled by its checkout whose first charge (49.00) fails stays Active, its
/// invoice errored and on the dunning ladder, as a renewal's would be.
/// Rules: R-105
#[rstest]
#[tokio::test]
async fn a_settled_trial_whose_first_charge_fails_goes_through_dunning(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::domain::InvoicePaymentStatus;
    use meteroid_store::domain::connectors::MockChargeBehavior;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;
    run(&env).await;
    env.set_now(at(1, 5, 12));
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            0,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout during the trial");
    drop(conn);
    run(&env).await;

    env.set_mock_charge_behavior(MockChargeBehavior::Failed)
        .await;
    env.set_now(at(1, 15, 1));
    run(&env).await;
    env.get_subscription(sub_id).await.assert().is_active();
    env.set_now(at(1, 16, 1));
    run(&env).await;
    env.run_outbox_and_orchestration().await;
    run(&env).await;

    let invoices = env.get_invoices(sub_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoices[0].payment_status, InvoicePaymentStatus::Errored);
    assert_eq!(
        (invoices[0].total, invoices[0].amount_due),
        (4900, 4900),
        "nothing collected"
    );
    assert_eq!(
        env.pending_payment_retries(sub_id, invoices[0].id).await,
        1,
        "one dunning retry"
    );
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_pending_checkout(false)
        .has_period_start(day(1, 15));
}

/// Decision 1, the other way: an OnCheckout free trial ended with its start unsettled is
/// TrialExpired, awaiting its checkout; with no card to collect, the controller writes nothing.
#[rstest]
#[tokio::test]
async fn a_free_trial_ended_unsettled_expires(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active()
        .has_pending_checkout(true);

    env.set_now(at(1, 15, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_expired()
        .has_pending_checkout(true)
        .has_period_start(day(1, 15));
    assert_eq!(invoices(&env, sub_id).await, vec![]);
    assert_eq!(
        count(&outbox(&env, sub_id).await, "updated Some(TrialEnded)"),
        1
    );
}

/// An OnCheckout free trial ended unsettled, its checkout never completed, the customer's card on
/// file: TrialExpired, its first invoice collected unattended (49.00), and the payment settles the
/// start: Active from then on.
#[rstest]
#[tokio::test]
async fn an_expired_trial_paid_from_a_card_on_file_is_active(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;
    run(&env).await;

    env.set_now(at(1, 15, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_expired()
        .has_pending_checkout(true);
    let opened = env.get_invoices(sub_id).await;
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].billing_period_start, day(1, 15));
    assert_eq!(opened[0].total, 4900);

    // Issued after its grace, then charged.
    env.set_now(at(1, 16, 1));
    run(&env).await;
    env.run_outbox_and_orchestration().await;
    let charged = env.get_invoices(sub_id).await;
    assert_eq!(charged.len(), 1);
    assert_eq!(
        (charged[0].total, charged[0].amount_due),
        (4900, 0),
        "paid in full"
    );
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(1, 15))
        .has_pending_checkout(false);
    env.set_now(at(1, 20, 1));
    run(&env).await;
    env.get_subscription(sub_id).await.assert().is_active();
}

/// An expired trial checked out with a card asking for 3DS: TrialExpired until the charge (49.00)
/// settles, then Active from the trial's end, never TrialActive again.
/// Rules: R-105
#[rstest]
#[tokio::test]
async fn an_expired_trial_checked_out_after_3ds_is_active(#[future] test_env: TestEnv) {
    use meteroid_store::domain::connectors::MockChargeBehavior;
    use meteroid_store::domain::enums::PaymentTypeEnum;
    use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

    let env = test_env.await;
    env.seed_payments().await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    run(&env).await;
    env.set_now(at(1, 20, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_expired();

    env.set_mock_charge_behavior(MockChargeBehavior::RequiresAction)
        .await;
    let mut conn = env.conn().await;
    env.services()
        .complete_subscription_checkout_tx(
            &mut conn,
            TENANT_ID,
            sub_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            4900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("the checkout awaits 3DS");
    drop(conn);
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_expired();

    let invoice = env.get_invoices(sub_id).await.remove(0);
    assert_eq!((invoice.total, invoice.amount_due), (4900, 4900));
    let pending = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice.id)
        .await
        .expect("transactions")
        .into_iter()
        .map(|detailed| detailed.transaction)
        .find(|tx| tx.payment_type == PaymentTypeEnum::Payment)
        .expect("the pending charge");
    env.deliver_mock_webhook(
        format!(
            r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{ext}","amount":4900,"currency":"EUR"}}"#,
            tx = pending.id.as_base62(),
            ext = pending.provider_transaction_id.clone().unwrap_or_default(),
        )
        .as_bytes(),
    )
    .await;
    env.run_outbox_and_orchestration().await;
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_pending_checkout(false)
        .has_period_start(day(1, 15));
    let paid = env.get_invoices(sub_id).await;
    assert_eq!(paid.len(), 1);
    assert_eq!((paid[0].total, paid[0].amount_due), (4900, 0));
}

/// Decision 3: a TrialExpired subscription's current period follows its grid while it waits for
/// its checkout: two months later it is the period holding the day, not the one the trial ended in.
#[rstest]
#[tokio::test]
async fn an_expired_trial_period_follows_the_grid(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(1, 1))
        .on_checkout()
        .trial_days(14)
        .no_auto_charge()
        .create(env.services())
        .await;
    env.set_now(at(1, 15, 1));
    run(&env).await;
    let expired = env.get_subscription(sub_id).await;
    expired.assert().is_trial_expired();
    assert_eq!(expired.current_period_start, day(1, 15));

    env.set_now(at(3, 20, 1));
    run(&env).await;
    let later = env.get_subscription(sub_id).await;
    later.assert().is_trial_expired().has_pending_checkout(true);
    let end = later.current_period_end.expect("a period on the grid");
    assert!(
        later.current_period_start > day(1, 15)
            && later.current_period_start <= day(3, 20)
            && day(3, 20) < end,
        "the period holding Mar 20: {}..{end}",
        later.current_period_start
    );
    assert!((28..=31).contains(&(end - later.current_period_start).num_days()));
    assert_eq!(invoices(&env, sub_id).await, vec![]);
}

fn credit_coupon(code: &str) -> CouponNew {
    CouponNew {
        code: code.to_owned(),
        description: "Welcome offer".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Credit {
            currency: "EUR".to_owned(),
            amount: dec!(25),
            expires_in_days: Some(30),
            scope: Default::default(),
        },
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: false,
        plan_ids: vec![],
    }
}

async fn coupon_grants(env: &TestEnv) -> usize {
    env.store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .expect("wallets")
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == CreditGrantSource::Coupon)
        .count()
}

/// Decision 2: pending_checkout is derived: an unsettled start whose checkout session is not
/// completed awaits it, before the start, during a free trial and after it alike. The promo
/// code's credit is not minted while it awaits its checkout (OQ9), its trial started or not.
/// Rules: OQ9
#[rstest]
#[tokio::test]
async fn pending_checkout_is_derived_from_the_start_and_the_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_mock_payment_provider(false).await;
    let coupon_id = env.create_coupon(credit_coupon("WELCOME25")).await;
    env.set_now(at(1, 10, 12));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID)
        .start_date(day(2, 1))
        .on_checkout()
        .trial_days(14)
        .no_auto_charge()
        .coupon(coupon_id)
        .create(env.services())
        .await;
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_pending_activation()
        .has_pending_checkout(true);
    assert_eq!(coupon_grants(&env).await, 0, "not started");

    env.set_now(at(2, 1, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active()
        .has_pending_checkout(true)
        .has_period_start(day(2, 1))
        .has_period_end(day(2, 15));
    assert_eq!(
        coupon_grants(&env).await,
        0,
        "a free trial awaiting its checkout mints nothing"
    );

    env.set_now(at(2, 15, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_expired()
        .has_pending_checkout(true);
    assert_eq!(coupon_grants(&env).await, 0, "still awaiting its checkout");
    assert_eq!(invoices(&env, sub_id).await, vec![]);
}

/// A paid trial bills from its start like any period; its end is a transition alone: Active with
/// a `TrialEnded` update on its day, no document, then the renewal bills the next month.
#[rstest]
#[tokio::test]
async fn a_paid_trial_ends_into_active_without_a_document(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 1));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_TRIAL_ID)
        .start_date(day(1, 1))
        .on_start()
        .trial_days(7)
        .no_auto_charge()
        .create(env.services())
        .await;
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active()
        .has_period_start(day(1, 1))
        .has_period_end(day(2, 1));
    let opening = vec!["Recurring 2024-01-01: Monthly Rate 2024-01-01..2024-02-01 9900"];
    assert_eq!(documents(&env, sub_id).await, opening);
    assert_eq!(
        count(&outbox(&env, sub_id).await, "updated Some(TrialEnded)"),
        0
    );

    env.set_now(at(1, 8, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(1, 1))
        .has_period_end(day(2, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        opening,
        "the trial's end bills nothing"
    );
    assert_eq!(
        count(&outbox(&env, sub_id).await, "updated Some(TrialEnded)"),
        1
    );

    env.set_now(at(2, 1, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Monthly Rate 2024-01-01..2024-02-01 9900",
            "Recurring 2024-02-01: Monthly Rate 2024-02-01..2024-03-01 9900",
        ]
    );
    assert_eq!(
        count(&outbox(&env, sub_id).await, "updated Some(TrialEnded)"),
        1
    );
}

/// A renewal, then the same run again at the same instant: nothing more is written, no event,
/// movement, outbox event or document, and the subscription's step is empty.
#[rstest]
#[tokio::test]
async fn a_renewal_run_again_writes_nothing(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    run(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    let renewed = env.get_subscription(sub_id).await;
    renewed
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_eq!(
        invoices(&env, sub_id).await,
        vec![
            (day(1, 1), InvoiceStatusEnum::Finalized),
            (day(2, 1), InvoiceStatusEnum::Finalized),
        ]
    );

    let snapshot = |row: diesel_models::subscriptions::SubscriptionRow| {
        (
            format!("{:?}", row.status),
            row.current_period_start,
            row.current_period_end,
            row.plan_version_id,
            row.mrr_cents,
            row.pending_checkout,
            row.next_wake_at,
        )
    };
    let before = (
        snapshot(renewed),
        documents(&env, sub_id).await,
        events(&env, sub_id).await,
        movements(&env, sub_id).await,
        outbox(&env, sub_id).await,
    );
    env.run_billing_controller().await;
    env.process_cycle_transitions().await;
    env.process_due_events().await;
    let step = env
        .services()
        .derive_subscription_state(TENANT_ID, sub_id, at(2, 1, 13))
        .await
        .expect("derive")
        .expect("a live subscription");
    assert!(
        step.transitions.is_empty() && step.state.is_none() && step.documents.is_empty(),
        "{step:?}"
    );
    let after = (
        snapshot(env.get_subscription(sub_id).await),
        documents(&env, sub_id).await,
        events(&env, sub_id).await,
        movements(&env, sub_id).await,
        outbox(&env, sub_id).await,
    );
    assert_eq!(after, before);
}

/// A daily controller and one three months late write the same: the same documents, invoice
/// statuses, state and movements.
#[tokio::test]
async fn a_daily_run_equals_one_late_run() {
    async fn subscribed() -> (TestEnv, SubscriptionId) {
        let env = test_env_with_seed(SeedLevel::PLANS).await;
        env.set_now(at(1, 15, 12));
        let sub_id = leetcode_from_jan_1(&env).await;
        run(&env).await;
        (env, sub_id)
    }
    let (daily, daily_sub) = subscribed().await;
    daily.run_until(day(4, 15)).await;
    daily.set_now(at(4, 15, 13));
    run(&daily).await;

    let (late, late_sub) = subscribed().await;
    late.set_now(at(4, 15, 13));
    run(&late).await;

    let state = |row: diesel_models::subscriptions::SubscriptionRow| {
        (
            format!("{:?}", row.status),
            row.current_period_start,
            row.current_period_end,
            row.mrr_cents,
            row.next_wake_at,
        )
    };
    assert_eq!(
        documents(&late, late_sub).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
            "Recurring 2024-03-01: Subscription Rate 2024-03-01..2024-04-01 3500",
            "Recurring 2024-04-01: Subscription Rate 2024-04-01..2024-05-01 3500",
        ]
    );
    assert_eq!(
        documents(&daily, daily_sub).await,
        documents(&late, late_sub).await
    );
    assert_eq!(
        invoices(&daily, daily_sub).await,
        invoices(&late, late_sub).await
    );
    assert_eq!(
        state(daily.get_subscription(daily_sub).await),
        state(late.get_subscription(late_sub).await)
    );
    assert_eq!(
        movements(&daily, daily_sub).await,
        movements(&late, late_sub).await
    );
    assert_eq!(
        events(&daily, daily_sub).await,
        events(&late, late_sub).await
    );
}

/// A cancellation still to come leaves the subscription Active and its period whole, billed
/// whole at the renewal; applied by a run two days late, it is stamped at its own date: Cancelled
/// from Feb 10, the unused 20 of February's 29 days given back, the churn and the Cancelled
/// outbox once.
/// Rules: R-077
#[rstest]
#[tokio::test]
async fn a_cancellation_to_come_then_applied(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            Some("too expensive".to_owned()),
            CancellationEffectiveAt::Date(day(2, 10)),
        )
        .await
        .expect("a cancellation on Feb 10");

    env.set_now(at(1, 20, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(1, 1))
        .has_period_end(day(2, 1));

    env.set_now(at(2, 1, 13));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
        ],
        "the period billed whole while its cancellation is to come"
    );
    assert_eq!(count(&outbox(&env, sub_id).await, "cancelled"), 0);

    env.set_now(at(2, 12, 1));
    run(&env).await;
    let cancelled = env.get_subscription(sub_id).await;
    cancelled
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled)
        .has_period_start(day(2, 10));
    assert_eq!(cancelled.current_period_end, None);
    assert_eq!(
        cancelled.terminated_at, None,
        "settled at the date's midnight"
    );
    // 3500 × 20/29 = 2413.79 → 2414.
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
            "note: Subscription Rate 2024-02-01..2024-03-01 -2414",
        ]
    );
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(2, 10))]
    );
    assert!(
        events(&env, sub_id)
            .await
            .contains(&("Cancelled".to_owned(), day(2, 10)))
    );
    let outbox = outbox(&env, sub_id).await;
    assert_eq!(count(&outbox, "cancelled"), 1);
    assert_eq!(count(&outbox, "updated Some(CancellationScheduled)"), 1);
    assert_eq!(cancelled.next_wake_at, None, "ended: never woken again");
}

/// An immediate cancellation on Feb 5 is effective when it returns, no controller run between:
/// Cancelled from that day, its churn and its outbox written, and February's unused days given
/// back, its day included (R-077): 3500 × 25/29 = 3017.24 → 3017.
#[rstest]
#[tokio::test]
async fn an_immediate_cancellation_is_effective_at_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    env.set_now(at(2, 5, 15));
    let returned = env
        .services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled at once");
    assert_eq!(returned.status, SubscriptionStatusEnum::Cancelled.into());
    let cancelled = env.get_subscription(sub_id).await;
    cancelled
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled)
        .has_period_start(day(2, 5));
    assert!(cancelled.terminated_at.is_some());
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(2, 5))]
    );
    assert_eq!(count(&outbox(&env, sub_id).await, "cancelled"), 1);
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
            "note: Subscription Rate 2024-02-01..2024-03-01 -3017",
        ]
    );
}

/// An immediate cancellation answers the documents its ending issued in the request: those the
/// subscription holds after it and not before, here the note giving back February's unused days
/// (3500 × 25/29 = 3017).
/// Rules: R-141, R-077
#[rstest]
#[tokio::test]
async fn an_immediate_cancellation_answers_its_documents(#[future] test_env: TestEnv) {
    use std::collections::BTreeSet;
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    let invoices_before: BTreeSet<_> = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .map(|invoice| invoice.id)
        .collect();

    env.set_now(at(2, 5, 15));
    let outcome = env
        .services()
        .cancel_subscription_with_documents(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled at once");
    assert!(!outcome.applies_later);
    assert_eq!(
        outcome.subscription.status,
        SubscriptionStatusEnum::Cancelled.into()
    );

    let invoices = env.get_invoices(sub_id).await;
    let issued: BTreeSet<_> = invoices
        .iter()
        .map(|invoice| invoice.id)
        .filter(|id| !invoices_before.contains(id))
        .collect();
    assert_eq!(
        outcome.invoice_ids.iter().copied().collect::<BTreeSet<_>>(),
        issued
    );
    let mut notes = Vec::new();
    for invoice in &invoices {
        for note in env
            .store()
            .list_credit_notes_by_invoice_id(TENANT_ID, invoice.id)
            .await
            .expect("credit notes")
        {
            notes.push((note.id, note.total));
        }
    }
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].1.abs(), 3017);
    assert_eq!(outcome.credit_note_ids, vec![notes[0].0]);
}

/// A cancellation asked while another is still to apply replaces it: one `Cancelled` event, moved
/// from Feb 1 to Feb 10, so February's renewal books no churn and the termination books one.
/// Asked once the subscription is cancelled, it is refused and writes nothing.
/// Rules: R-141
#[rstest]
#[tokio::test]
async fn a_second_cancellation_moves_the_first(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    for effective_at in [
        CancellationEffectiveAt::EndOfBillingPeriod,
        CancellationEffectiveAt::Date(day(2, 10)),
    ] {
        env.services()
            .cancel_subscription(Actor::System, sub_id, TENANT_ID, None, effective_at)
            .await
            .expect("a cancellation to come");
    }
    let cancellations = |events: Vec<(String, NaiveDate)>| -> Vec<(String, NaiveDate)> {
        events
            .into_iter()
            .filter(|(kind, _)| kind == "Cancelled")
            .collect()
    };
    assert_eq!(
        cancellations(events(&env, sub_id).await),
        vec![("Cancelled".to_owned(), day(2, 10))],
        "one event, moved"
    );

    env.set_now(at(2, 1, 13));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(2, 1));
    assert!(
        of_kind(&movements(&env, sub_id).await, "Churn").is_empty(),
        "the renewal books no churn"
    );

    env.set_now(at(2, 12, 1));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled)
        .has_period_start(day(2, 10));
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(2, 10))]
    );

    let again = env
        .services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await;
    let Err(error) = again else {
        panic!("an ended subscription is cancelled again");
    };
    assert!(
        matches!(
            error.current_context(),
            meteroid_store::errors::StoreError::Conflict(_)
        ),
        "{error:?}"
    );
    assert_eq!(cancellations(events(&env, sub_id).await).len(), 1);
    assert_eq!(of_kind(&movements(&env, sub_id).await, "Churn").len(), 1);
    assert_eq!(count(&outbox(&env, sub_id).await, "cancelled"), 1);
}

/// A cancellation revoked before its date leaves no `Cancelled` event: the subscription renews on
/// the day it would have ended, books no churn and keeps its MRR.
/// Rules: R-141
#[rstest]
#[tokio::test]
async fn a_revoked_cancellation_books_no_churn(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::EndOfBillingPeriod,
        )
        .await
        .expect("a cancellation on Feb 1");
    assert!(
        events(&env, sub_id)
            .await
            .contains(&("Cancelled".to_owned(), day(2, 1)))
    );
    let ids = [sub_id];
    let pending =
        SubscriptionEndingRow::list_pending_cancellations(&mut env.conn().await, TENANT_ID, &ids)
            .await
            .expect("pending cancellations");
    let [pending] = pending.as_slice() else {
        panic!("one cancellation to come: {pending:?}");
    };
    env.services()
        .cancel_scheduled_event(
            Actor::System,
            common_domain::ids::ScheduledEventId::from(pending.id),
            sub_id,
            TENANT_ID,
        )
        .await
        .expect("revoked");
    assert!(
        !events(&env, sub_id)
            .await
            .iter()
            .any(|(kind, _)| kind == "Cancelled"),
        "the revoked cancellation's event goes with it"
    );

    env.set_now(at(2, 1, 13));
    run(&env).await;
    let renewed = env.get_subscription(sub_id).await;
    renewed
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert!(of_kind(&movements(&env, sub_id).await, "Churn").is_empty());
    assert_eq!(renewed.mrr_cents, 3500);
}

/// An immediate cancellation whose step panics once its transaction committed is recorded all the
/// same: the request answers the subscription as it stands, the step is the subscription's
/// failure, and the controller's retry applies the ending at its instant: one `Cancelled` event,
/// one churn, one webhook, February's unused days given back once.
/// Rules: R-141, R-121
#[cfg(debug_assertions)]
#[rstest]
#[tokio::test]
async fn a_cancellation_failing_after_its_commit_is_applied_by_the_controller(
    #[future] test_env: TestEnv,
) {
    use meteroid_store::services::step_faults;
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;

    env.set_now(at(2, 5, 15));
    step_faults::panic_in_steps_of(sub_id);
    let returned = env
        .services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await;
    step_faults::clear();
    let returned = returned.expect("committed, a cancellation never fails its request");
    assert_eq!(
        returned.status,
        SubscriptionStatusEnum::Active.into(),
        "its step did not run"
    );
    let failed = env.get_subscription(sub_id).await;
    assert_eq!(failed.billing_failures, 1);
    assert!(of_kind(&movements(&env, sub_id).await, "Churn").is_empty());

    env.set_now(at(2, 5, 16));
    run(&env).await;
    let cancelled = env.get_subscription(sub_id).await;
    cancelled
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled)
        .has_period_start(day(2, 5));
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(2, 5))]
    );
    assert_eq!(
        events(&env, sub_id)
            .await
            .iter()
            .filter(|(kind, _)| kind == "Cancelled")
            .count(),
        1
    );
    assert_eq!(count(&outbox(&env, sub_id).await, "cancelled"), 1);
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
            "note: Subscription Rate 2024-02-01..2024-03-01 -3017",
        ]
    );
}

/// Decision 7: an immediate cancellation is stamped at its instant however late the controller
/// applies it: `terminated_at` the instant, the period ending that day, the churn dated it.
#[rstest]
#[tokio::test]
async fn a_transition_is_stamped_at_its_facts_instant(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    env.set_now(at(2, 5, 15));
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled at once");
    let ending = SubscriptionEndingRow::list_of(&mut env.conn().await, sub_id)
        .await
        .expect("endings")
        .into_iter()
        .find(|ending| ending.kind == diesel_models::subscription_facts::ENDING_CANCEL)
        .expect("the cancellation fact")
        .at;
    assert_eq!(ending.date(), day(2, 5));
    assert!(
        ending > day(2, 5).and_hms_opt(15, 0, 0).unwrap(),
        "its instant"
    );

    env.set_now(at(2, 7, 1));
    run(&env).await;
    let cancelled = env.get_subscription(sub_id).await;
    cancelled
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled)
        .has_period_start(day(2, 5));
    assert_eq!(cancelled.terminated_at, Some(ending));
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(2, 5))]
    );
    assert_eq!(count(&outbox(&env, sub_id).await, "cancelled"), 1);
}

/// A fixed term completes at its end date: Completed from Mar 1, its Ended event and churn dated
/// then, the Ended outbox once, no March invoice.
#[rstest]
#[tokio::test]
async fn a_fixed_term_completes_at_its_end(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .end_date(day(3, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));

    env.set_now(at(3, 1, 1));
    run(&env).await;
    let completed = env.get_subscription(sub_id).await;
    completed
        .assert()
        .has_status(SubscriptionStatusEnum::Completed)
        .has_period_start(day(3, 1));
    assert_eq!(completed.current_period_end, None);
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
        ]
    );
    assert!(
        events(&env, sub_id)
            .await
            .contains(&("Ended".to_owned(), day(3, 1)))
    );
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(3, 1))]
    );
    assert_eq!(count(&outbox(&env, sub_id).await, "ended"), 1);
}

/// A fixed term to Mar 1 cancelled at the period end, Mar 1: the term ends it, Completed, its
/// Ended event and churn dated then, never Cancelled; the derived state agrees with the stored one.
#[rstest]
#[tokio::test]
async fn a_cancellation_at_a_fixed_terms_end_completes_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(day(1, 1))
        .end_date(day(3, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::EndOfBillingPeriod,
        )
        .await
        .expect("cancelled at the period end");
    run(&env).await;

    env.set_now(at(3, 1, 1));
    run(&env).await;
    env.set_now(at(3, 2, 1));
    run(&env).await;
    let ended = env.get_subscription(sub_id).await;
    ended
        .assert()
        .has_status(SubscriptionStatusEnum::Completed)
        .has_period_start(day(3, 1));
    assert_eq!(ended.current_period_end, None);
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500",
            "Recurring 2024-02-01: Subscription Rate 2024-02-01..2024-03-01 3500",
        ]
    );
    assert!(
        events(&env, sub_id)
            .await
            .contains(&("Ended".to_owned(), day(3, 1)))
    );
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Churn"),
        vec![("Churn".to_owned(), -3500, day(3, 1))]
    );
}

/// Decision 5: a plan window changing at the period end and the renewal are one evaluation: a
/// single subscription step applies Pro (its Switch row, PlanChanged, the MRR's Expansion) and
/// bills Pro's February.
/// Rules: R-084, R-011
#[rstest]
#[tokio::test]
async fn a_plan_change_and_its_renewal_are_one_step(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
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
        .expect("Pro at the period end");
    run(&env).await;
    assert_eq!(
        env.get_subscription(sub_id).await.plan_version_id,
        PLAN_VERSION_STARTER_ID
    );

    env.set_now(at(2, 1, 1));
    env.process_cycle_transitions().await;
    let renewed = env.get_subscription(sub_id).await;
    assert_eq!(renewed.plan_version_id, PLAN_VERSION_PRO_ID);
    renewed
        .assert()
        .is_active()
        .has_period_start(day(2, 1))
        .has_period_end(day(3, 1));
    assert_eq!(
        documents(&env, sub_id).await,
        vec![
            "Recurring 2024-01-01: Platform Fee 2024-01-01..2024-02-01 2900",
            "Recurring 2024-01-01: Seats 2024-01-01..2024-02-01 1000",
            "Recurring 2024-02-01: Platform Fee 2024-02-01..2024-03-01 9900",
            "Recurring 2024-02-01: Seats 2024-02-01..2024-03-01 2500",
        ]
    );
    assert!(
        events(&env, sub_id)
            .await
            .contains(&("Switch".to_owned(), day(2, 1)))
    );
    // 9900 + 2500 − (2900 + 1000).
    assert_eq!(
        of_kind(&movements(&env, sub_id).await, "Expansion"),
        vec![("Expansion".to_owned(), 8500, day(2, 1))]
    );
    assert_eq!(
        count(&outbox(&env, sub_id).await, "updated Some(PlanChanged)"),
        1
    );
}

/// Decision 5: a cancellation dated inside the period and the renewal after it are one
/// evaluation, with no wait for the cancellation to apply first: one step, late, cancels on
/// Jan 31 and bills no February.
#[rstest]
#[tokio::test]
async fn a_cancellation_in_the_period_and_the_renewal_are_one_step(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(1, 31)),
        )
        .await
        .expect("a cancellation on Jan 31");

    env.set_now(at(2, 2, 1));
    env.process_cycle_transitions().await;
    let cancelled = env.get_subscription(sub_id).await;
    cancelled
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled)
        .has_period_start(day(1, 31));
    assert!(
        invoices(&env, sub_id)
            .await
            .iter()
            .all(|(start, _)| *start < day(2, 1)),
        "February is never billed"
    );
    assert_eq!(count(&outbox(&env, sub_id).await, "cancelled"), 1);
}

/// Decision 6: a subscription on a free plan whose every charge is zero writes no document, at its
/// start or its renewals, as a free plan never billed; its state still renews. A standard plan priced at
/// nothing is as free.
/// Rules: R-104
#[rstest]
#[case::typed_free(true)]
#[case::priced_at_nothing(false)]
#[tokio::test]
async fn a_free_plan_billing_nothing_writes_no_document(
    #[future] test_env: TestEnv,
    #[case] typed_free: bool,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let version = PlanVersionId::new();
    let seed = PlanSeed::new(PlanId::new(), "Free at nothing", version);
    let seed = if typed_free { seed.free() } else { seed };
    seed.components(vec![SeedComp::rate(
        PriceComponentId::new(),
        "Free Rate",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        Decimal::ZERO,
    )])
    .seed(&mut env.conn().await)
    .await
    .expect("a free plan");
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert!(
        invoices(&env, sub_id).await.is_empty(),
        "no opening invoice"
    );
    run(&env).await;
    assert!(invoices(&env, sub_id).await.is_empty(), "none at its start");

    env.set_now(at(3, 1, 13));
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_active()
        .has_period_start(day(3, 1))
        .has_period_end(day(4, 1));
    assert!(
        invoices(&env, sub_id).await.is_empty(),
        "no renewal written"
    );
}

/// A free plan's unwritten periods stay unwritten once it is changed to a paid plan: the change
/// bills its own days, no zero invoice is back-dated to the free months.
/// Rules: R-104
#[rstest]
#[tokio::test]
async fn a_free_plan_changed_to_a_paid_one_back_dates_no_zero_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Free at nothing", version)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Free Rate",
            PRODUCT_PLATFORM_FEE_ID,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            Decimal::ZERO,
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a free plan");
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    run(&env).await;
    env.set_now(at(3, 10, 12));
    run(&env).await;
    assert!(
        invoices(&env, sub_id).await.is_empty(),
        "no free month written"
    );

    env.services()
        .apply_plan_change_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_PRO_ID,
            vec![],
            day(3, 10),
        )
        .await
        .expect("an immediate upgrade");
    run(&env).await;
    let zero: Vec<_> = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.total == 0)
        .map(|invoice| invoice.billing_period_start)
        .collect();
    assert!(zero.is_empty(), "zero invoices back-dated to {zero:?}");
    assert!(
        !env.get_invoices(sub_id).await.is_empty(),
        "the upgrade bills"
    );
}

/// A free plan's included credit renews each period: its zero invoice, which grants it, is
/// written at every renewal, as on a plan priced at nothing.
/// Rules: R-104
#[rstest]
#[case::typed_free(true)]
#[case::priced_at_nothing(false)]
#[tokio::test]
async fn a_free_plans_included_credit_renews(
    #[future] test_env: TestEnv,
    #[case] typed_free: bool,
) {
    use meteroid_store::domain::prices::{FeeStructure, Pricing};
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let version = PlanVersionId::new();
    let mut credits = SeedComp::rate(
        PriceComponentId::new(),
        "Included credits",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        Decimal::ZERO,
    )
    .with_structure(FeeStructure::Credit {});
    credits.prices[0].pricing = Pricing::Credit {
        unit_price: Decimal::ZERO,
        quantity: 1,
        credit_amount: dec!(500),
        expiry: None,
        credit_unit_id: None,
        rollover: None,
        once: false,
    };
    let seed = PlanSeed::new(PlanId::new(), "Free with credits", version);
    let seed = if typed_free { seed.free() } else { seed };
    seed.components(vec![credits])
        .seed(&mut env.conn().await)
        .await
        .expect("a free plan including credit");
    let sub_id = subscription()
        .customer(CUST_UBER_ID)
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    run(&env).await;
    env.set_now(at(2, 5, 12));
    run(&env).await;
    env.run_outbox_and_orchestration().await;

    let written = invoices(&env, sub_id).await;
    assert_eq!(
        written.iter().map(|(start, _)| *start).collect::<Vec<_>>(),
        vec![day(1, 1), day(2, 1)],
        "the renewal grants it again"
    );
    let granted: i64 = env
        .store()
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .expect("wallets")
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == CreditGrantSource::Included)
        .map(|grant| grant.amount_minor)
        .sum();
    assert_eq!(granted, 100_000, "500 a month for two months");
}

/// A paid plan's period of zero charges still gets its invoice: only a free plan's is skipped.
/// Rules: R-104
#[rstest]
#[tokio::test]
async fn a_paid_plans_period_of_zero_charges_still_gets_its_invoice(#[future] test_env: TestEnv) {
    use meteroid_store::domain::UsagePricingModel;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Pay as you go", version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::ZERO,
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Bandwidth",
                PRODUCT_BANDWIDTH_ID,
                METRIC_BANDWIDTH,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit { rate: dec!(0.10) },
            ),
        ])
        .seed(&mut env.conn().await)
        .await
        .expect("a paid plan");
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    assert_eq!(
        billed(&env, sub_id).await,
        vec![(
            meteroid_store::domain::enums::InvoiceType::Recurring,
            day(1, 1),
            0
        )],
        "its opening period is invoiced at zero"
    );
}

/// A free plan billing bandwidth at 0.10 a unit, 100 units in any window read.
async fn free_plan_with_usage() -> (TestEnv, PlanVersionId) {
    use meteroid_store::clients::usage::{GroupedUsageData, MutableMockUsageClient, UsageData};
    use meteroid_store::domain::{UsagePeriod, UsagePricingModel};
    let usage = MutableMockUsageClient::new();
    let env = crate::harness::test_env_with_seed_and_usage(
        SeedLevel::PLANS,
        std::sync::Arc::new(usage.clone()),
    )
    .await;
    let midnight = day(1, 1).and_time(chrono::NaiveTime::MIN);
    usage.set_usage_for_any_nonempty_window(
        METRIC_BANDWIDTH,
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(100),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod {
                start: midnight,
                end: midnight,
            },
        },
    );
    let version = PlanVersionId::new();
    PlanSeed::new(PlanId::new(), "Free with usage", version)
        .free()
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit { rate: dec!(0.10) },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a free plan");
    (env, version)
}

/// The subscription's invoices as (type, period start, total), in order.
async fn billed(
    env: &TestEnv,
    sub_id: SubscriptionId,
) -> Vec<(meteroid_store::domain::enums::InvoiceType, NaiveDate, i64)> {
    let mut billed: Vec<_> = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .map(|invoice| {
            (
                invoice.invoice_type,
                invoice.billing_period_start,
                invoice.total,
            )
        })
        .collect();
    billed.sort_by_key(|(_, start, _)| *start);
    billed
}

/// A free plan's metered fee is a priced charge: January's 100 units at 0.10 are billed on Feb 1,
/// 1000, as the previous release billed every renewal of a free plan.
/// Rules: R-104
#[tokio::test]
async fn a_free_plans_metered_fee_bills_its_usage() {
    let (env, version) = free_plan_with_usage().await;
    env.set_now(at(1, 15, 12));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    run(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    assert_eq!(
        billed(&env, sub_id).await,
        vec![(
            meteroid_store::domain::enums::InvoiceType::Recurring,
            day(2, 1),
            1000
        )]
    );
}

/// A free plan cancelled at once mid-period bills the usage to its ending, 100 units or 1000, on
/// the document closing it: final invoices off, the recurring invoice of the ending's day.
/// Rules: R-104, R-016
#[tokio::test]
async fn a_free_plan_cancelled_at_once_bills_its_usage_to_the_ending() {
    let (env, version) = free_plan_with_usage().await;
    env.set_now(at(1, 15, 12));
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    run(&env).await;
    env.set_now(at(1, 20, 10));
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Immediate,
        )
        .await
        .expect("cancelled at once");
    run(&env).await;
    env.get_subscription(sub_id)
        .await
        .assert()
        .has_status(SubscriptionStatusEnum::Cancelled);
    assert_eq!(
        billed(&env, sub_id).await,
        vec![(
            meteroid_store::domain::enums::InvoiceType::Recurring,
            day(1, 20),
            1000
        )]
    );
}

/// A plan change the previous release left pending as a scheduled event, dated `on`, replacing
/// every fee by `fee` priced at `price`, written as facts by the release's converter, which leaves
/// its documents to the controller.
async fn converted_plan_change(
    env: &TestEnv,
    sub_id: SubscriptionId,
    to: (PlanVersionId, PriceComponentId, PriceId),
    fee: Decimal,
    on: NaiveDate,
) {
    use meteroid_store::domain::enums::SubscriptionFeeBillingPeriod;
    use meteroid_store::domain::scheduled_events::ComponentMapping;
    use meteroid_store::domain::subscription_components::SubscriptionFee;
    let mut conn = env.conn().await;
    let current: Vec<common_domain::ids::SubscriptionPriceComponentId> = {
        use diesel_models::schema::subscription_component::dsl as c;
        c::subscription_component
            .filter(c::subscription_id.eq(sub_id))
            .filter(c::effective_to.is_null())
            .select(c::id)
            .load(&mut conn)
            .await
            .expect("the open rows")
    };
    let mut mappings: Vec<ComponentMapping> = current
        .into_iter()
        .map(|current_component_id| ComponentMapping::Removed {
            current_component_id,
        })
        .collect();
    mappings.push(ComponentMapping::Added {
        target_component_id: to.1,
        product_id: Some(PRODUCT_PLATFORM_FEE_ID),
        price_id: Some(to.2),
        name: "Platform Fee".to_owned(),
        fee: SubscriptionFee::Rate { rate: fee },
        period: SubscriptionFeeBillingPeriod::Monthly,
        seats: None,
    });
    let data = serde_json::json!({
        "ApplyPlanChange": {
            "new_plan_version_id": to.0,
            "component_mappings": mappings,
        }
    });
    diesel::sql_query(
        "CREATE TABLE scheduled_event (
             id UUID PRIMARY KEY, subscription_id UUID NOT NULL, tenant_id UUID NOT NULL,
             event_type TEXT NOT NULL, scheduled_time TIMESTAMP NOT NULL, priority INTEGER NOT NULL,
             event_data JSONB NOT NULL, created_at TIMESTAMP NOT NULL,
             updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, status TEXT NOT NULL,
             error TEXT, source TEXT NOT NULL, created_by_customer BOOLEAN NOT NULL DEFAULT false,
             actor_type \"ActorTypeEnum\" NOT NULL DEFAULT 'SYSTEM', actor_uuid UUID,
             actor_alias TEXT)",
    )
    .execute(&mut conn)
    .await
    .expect("main's scheduled_event");
    diesel::sql_query(
        "INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time,
             priority, event_data, created_at, status, source)
         VALUES ($1, $2, $3, 'APPLY_PLAN_CHANGE', $4, 0, $5, now(), 'PENDING', 'api')",
    )
    .bind::<diesel::sql_types::Uuid, _>(uuid::Uuid::now_v7())
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(TENANT_ID.as_uuid())
    .bind::<diesel::sql_types::Timestamp, _>(on.and_time(chrono::NaiveTime::MIN))
    .bind::<diesel::sql_types::Jsonb, _>(data)
    .execute(&mut conn)
    .await
    .expect("a pending plan change");
    meteroid_store::services::facts::pending::write_pending_scheduled_changes(&mut conn)
        .await
        .expect("converted")
        .expect("written as facts");
    // The release drops the table and leaves every subscription to evaluate.
    diesel::sql_query("DROP TABLE scheduled_event")
        .execute(&mut conn)
        .await
        .expect("dropped");
    diesel::sql_query("UPDATE subscription SET next_wake_at = NULL WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
        .execute(&mut conn)
        .await
        .expect("no wake yet");
}

/// A change the previous release left pending for a day inside a period, written as facts by the
/// release, bills its stub and gives back its unused days when its day comes, as the earlier
/// release applied it: LeetCode 35.00 to 60.00 on Jan 16, 16 of 31 days left, 3097 on an
/// adjustment invoice and 1806 of January's 3500 given back; once, whatever the runs after. The
/// release on Jan 10 (the change still to come) or on Jan 20 (due already, the controller late).
/// Rules: R-077, R-080
#[rstest]
#[case::still_to_come(at(1, 10, 12), at(1, 16, 12))]
#[case::due_already(at(1, 20, 12), at(1, 20, 13))]
#[tokio::test]
async fn a_converted_change_inside_its_period_bills_its_stub(
    #[future] test_env: TestEnv,
    #[case] released: DateTime<Utc>,
    #[case] runs: DateTime<Utc>,
) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    let sixty = (
        PlanVersionId::new(),
        PriceComponentId::new(),
        PriceId::new(),
    );
    PlanSeed::new(PlanId::new(), "Sixty", sixty.0)
        .components(vec![SeedComp::rate(
            sixty.1,
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            sixty.2,
            DieselBillingPeriodEnum::Monthly,
            dec!(60),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a 60.00 plan");
    run(&env).await;
    env.set_now(released);
    converted_plan_change(&env, sub_id, sixty, dec!(60), day(1, 16)).await;
    run(&env).await;

    env.set_now(runs);
    run(&env).await;
    let expected = vec![
        "Recurring 2024-01-01: Subscription Rate 2024-01-01..2024-02-01 3500".to_string(),
        "note: Subscription Rate 2024-01-01..2024-02-01 -1806".to_string(),
        "Adjustment 2024-01-16: Platform Fee (prorated) 2024-01-16..2024-02-01 3097".to_string(),
    ];
    assert_eq!(documents(&env, sub_id).await, expected);
    env.set_now(at(1, 25, 12));
    run(&env).await;
    assert_eq!(documents(&env, sub_id).await, expected, "settled once");
}

/// A plan of its own at 20.00 a month whose price a test can break: every step of a subscription
/// on it fails while broken.
async fn breakable_plan(env: &TestEnv) -> (PlanVersionId, PriceId) {
    let (version, price) = (PlanVersionId::new(), PriceId::new());
    PlanSeed::new(PlanId::new(), "Breakable", version)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform Fee",
            PRODUCT_PLATFORM_FEE_ID,
            price,
            DieselBillingPeriodEnum::Monthly,
            dec!(20),
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a plan");
    (version, price)
}

/// Rewrites `price`'s rates into something no fee reads, past the trigger refusing it.
async fn break_price(env: &TestEnv, price: PriceId) {
    let mut conn = env.conn().await;
    for sql in [
        "ALTER TABLE price DISABLE TRIGGER trg_price_rates_are_dated".to_string(),
        format!(
            "UPDATE price SET pricing = '{{\"type\": \"Broken\"}}' WHERE id = '{}'",
            price.as_uuid()
        ),
        "ALTER TABLE price ENABLE TRIGGER trg_price_rates_are_dated".to_string(),
    ] {
        diesel::sql_query(sql)
            .execute(&mut conn)
            .await
            .expect("broken price");
    }
}

/// The customer's controller row: (attempts, last error).
async fn controller_row(env: &TestEnv) -> (i32, Option<String>) {
    use diesel_models::schema::billing_controller::dsl as b;
    b::billing_controller
        .filter(b::tenant_id.eq(TENANT_ID))
        .filter(b::customer_id.eq(CUST_UBER_ID))
        .select((b::attempts, b::last_error))
        .first(&mut env.conn().await)
        .await
        .expect("the customer's controller row")
}

/// A subscription whose step fails is backed off on its own; the customer's run completes, so its
/// other subscription renews at its boundary: Feb 1's 3500 written by the first run past midnight.
/// Rules: R-121
#[rstest]
#[tokio::test]
async fn a_failing_subscription_leaves_its_siblings_on_time(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let (version, price) = breakable_plan(&env).await;
    let failing = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let sibling = leetcode_from_jan_1(&env).await;
    run(&env).await;

    break_price(&env, price).await;
    env.set_now(at(1, 31, 23));
    diesel::sql_query("UPDATE subscription SET next_wake_at = $1 WHERE id = $2")
        .bind::<diesel::sql_types::Timestamptz, _>(at(1, 31, 23))
        .bind::<diesel::sql_types::Uuid, _>(failing.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("due");
    env.run_billing_controller().await;
    assert_eq!(
        controller_row(&env).await,
        (0, None),
        "the customer did not fail"
    );
    let failure = env.get_subscription(failing).await;
    assert_eq!(failure.billing_failures, 1);
    assert!(failure.billing_last_error.is_some());
    assert_eq!(
        failure
            .next_wake_at
            .map(|wake| (wake - at(1, 31, 23)).num_minutes()),
        Some(4),
        "backed off on its own"
    );

    env.set_now(at(2, 1, 0) + chrono::Duration::minutes(1));
    env.run_billing_controller().await;
    assert!(
        invoices(&env, sibling)
            .await
            .contains(&(day(2, 1), InvoiceStatusEnum::Draft)),
        "the sibling's renewal is written at its boundary"
    );
}

/// A subscription failing again and again backs off exponentially up to an hour for its first
/// five attempts, then retries daily, forever; from its fifth failure the API shows it, and a
/// step taking again clears it.
/// Rules: R-121
#[rstest]
#[tokio::test]
async fn a_subscription_failing_five_times_shows_its_failure(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::SubscriptionInterface;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let (version, price) = breakable_plan(&env).await;
    let failing = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    run(&env).await;
    break_price(&env, price).await;
    diesel::sql_query("UPDATE subscription SET next_wake_at = $1 WHERE id = $2")
        .bind::<diesel::sql_types::Timestamptz, _>(at(1, 2, 0))
        .bind::<diesel::sql_types::Uuid, _>(failing.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("due");

    let mut now = at(1, 2, 0);
    let mut waits = Vec::new();
    for attempt in 1..=7 {
        env.set_now(now);
        env.run_billing_controller().await;
        let row = env.get_subscription(failing).await;
        assert_eq!(row.billing_failures, attempt);
        let retry = row.next_wake_at.expect("a retry");
        waits.push((retry - now).num_minutes());
        let shown = env
            .store()
            .get_subscription(TENANT_ID, failing)
            .await
            .expect("the subscription");
        if attempt < 5 {
            assert_eq!((shown.error_count, shown.next_retry), (0, None));
        } else {
            assert_eq!(shown.error_count, attempt);
            assert!(shown.last_error.is_some());
            assert_eq!(shown.next_retry, Some(retry.naive_utc()));
        }
        now = retry;
    }
    assert_eq!(waits, vec![4, 8, 16, 32, 60, 1440, 1440]);

    // Mended: the next retry takes, and nothing of the failure stays.
    let mut conn = env.conn().await;
    for sql in [
        "ALTER TABLE price DISABLE TRIGGER trg_price_rates_are_dated".to_string(),
        format!(
            "UPDATE price SET pricing = '{{\"rate\": \"20\", \"type\": \"Rate\"}}' WHERE id = '{}'",
            price.as_uuid()
        ),
        "ALTER TABLE price ENABLE TRIGGER trg_price_rates_are_dated".to_string(),
    ] {
        diesel::sql_query(sql)
            .execute(&mut conn)
            .await
            .expect("mended");
    }
    env.set_now(now);
    env.run_billing_controller().await;
    let row = env.get_subscription(failing).await;
    assert_eq!((row.billing_failures, row.billing_last_error), (0, None));
    let shown = env
        .store()
        .get_subscription(TENANT_ID, failing)
        .await
        .expect("the subscription");
    assert_eq!(shown.error_count, 0);
}

/// A step that panics (a fault the tests inject into its subscription's steps) is that
/// subscription's failure: the controller goes on, the customer's other subscription renews, and
/// the next pass runs.
/// Rules: R-121
#[cfg(debug_assertions)]
#[tokio::test]
async fn a_panicking_step_is_its_subscriptions_failure() {
    use meteroid_store::services::step_faults;
    let (env, _) = free_plan_with_usage().await;
    env.set_now(at(1, 1, 12));
    let panicking = leetcode_from_jan_1(&env).await;
    let sibling = leetcode_from_jan_1(&env).await;
    run(&env).await;

    step_faults::panic_in_steps_of(panicking);
    env.set_now(at(2, 1, 1));
    env.run_billing_controller().await;
    step_faults::clear();
    let failed = env.get_subscription(panicking).await;
    assert_eq!(failed.billing_failures, 1);
    assert!(
        failed
            .billing_last_error
            .as_deref()
            .is_some_and(|error| error.contains("panicked")),
        "{:?}",
        failed.billing_last_error
    );
    assert_eq!(controller_row(&env).await, (0, None));
    assert!(
        invoices(&env, sibling)
            .await
            .iter()
            .any(|(start, _)| *start == day(2, 1)),
        "the sibling renews"
    );
}

/// Decision 8: a written slot is never due again: a voided renewal is not billed again by the
/// next runs (R-096), the period keeps its one, voided, invoice.
/// Rules: R-096
#[rstest]
#[tokio::test]
async fn a_written_slot_is_never_written_again(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = leetcode_from_jan_1(&env).await;
    env.set_now(at(2, 1, 13));
    run(&env).await;
    let february = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(2, 1))
        .expect("February's renewal");
    env.store()
        .void_invoice(Actor::System, february.id, TENANT_ID)
        .await
        .expect("voided");

    env.set_now(at(2, 2, 1));
    run(&env).await;
    env.process_cycle_transitions().await;
    let step = env
        .services()
        .derive_subscription_state(TENANT_ID, sub_id, at(2, 2, 1))
        .await
        .expect("derive")
        .expect("a live subscription");
    assert!(step.documents.is_empty(), "{step:?}");
    assert_eq!(
        invoices(&env, sub_id).await,
        vec![
            (day(1, 1), InvoiceStatusEnum::Finalized),
            (day(2, 1), InvoiceStatusEnum::Void),
        ]
    );
}
