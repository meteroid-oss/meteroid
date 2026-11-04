//! Lock-order stress: the billing operations that lock a customer race on one customer and one
//! subscription. Business refusals are expected outcomes of a race; a deadlock, a lock timeout
//! or a hang is a lock-order bug.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CheckoutSessionId, CreditNoteId, CustomerConnectionId, CustomerId,
    CustomerPaymentMethodId, InvoiceId, PaymentTransactionId, SubscriptionId,
};
use diesel_models::enums::SubscriptionStatusEnum;
use http::HeaderMap;
use rstest::rstest;
use tokio::sync::{Barrier, watch};

use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::domain::connectors::{
    Connector, ConnectorConfig, CredentialState, MockChargeBehavior, MockPublicData,
    MockSensitiveData, PaymentConnectorConfig, PaymentProviderConfig,
};
use meteroid_store::domain::credit::CreditApplication;
use meteroid_store::domain::enums::{
    ConnectorStatusEnum, CustomerType, InvoiceStatusEnum, ReversalKind,
};
use meteroid_store::domain::{
    CreateCheckoutSession, CustomerNew, CustomerTaxStatus, Invoice, PaymentStatusEnum,
    SlotUpgradeBillingMode,
};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditType};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{CreditNoteInterface, CustomersInterface};
use meteroid_store::services::ReverseManualPaymentCommand;
use meteroid_store::{Services, Store};

use crate::data::ids::*;
use crate::data::payment;
use crate::harness::{TestEnv, subscription, test_env};
use crate::helpers::credit::{Grant, assert_ledger_balances, fund_subscription_grant};

const ITERATIONS: usize = 2 * OP_COUNT;
const OP_TIMEOUT: Duration = Duration::from_secs(60);
const HEAD_START_STEP: Duration = Duration::from_millis(4);
const CURRENCY: &str = "EUR";
const LOCK_FAILURES: [&str; 3] = ["deadlock detected", "lock timeout", "could not obtain lock"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Finalize,
    PlanChange,
    Seats,
    Settlement,
    Worker,
    CreditNoteVoid,
    Checkout,
    Chargeback,
}

const OP_COUNT: usize = 8;
const OPS: [Op; OP_COUNT] = [
    Op::Finalize,
    Op::PlanChange,
    Op::Seats,
    Op::Settlement,
    Op::Worker,
    Op::CreditNoteVoid,
    Op::Checkout,
    Op::Chargeback,
];

struct Race {
    customer_id: CustomerId,
    payment_method_id: CustomerPaymentMethodId,
    subscription_id: SubscriptionId,
    cycle_index_before: Option<i32>,
    draft_invoice_id: InvoiceId,
    change_date: NaiveDate,
    seats_at: NaiveDateTime,
    settled_invoice_id: InvoiceId,
    webhook_payload: Vec<u8>,
    credit_note_id: CreditNoteId,
    checkout_session_id: CheckoutSessionId,
    checkout_amount: u64,
    wire_id: PaymentTransactionId,
}

/// Releases the followers when the leader is done, also when its task panics.
struct ReleaseOnDrop(watch::Sender<bool>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct Raced {
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    checkout_completed: bool,
}

/// Finalization, an immediate plan change, a seat upgrade, a payment settlement, the lifecycle
/// worker, a credit-note void, a checkout and a payment recall, racing on one customer and one
/// subscription, never deadlock nor hang, each wins once, and the ledger and invoices stay consistent.
#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_billing_on_one_customer_never_deadlocks(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let mut succeeded = [0_usize; OP_COUNT];
    let mut raced = Vec::with_capacity(ITERATIONS);
    for iteration in 0..ITERATIONS {
        let race = Arc::new(prepare(&env, iteration).await);
        // Each operation leads in turn and completes before the other seven race: a head start
        // alone does not decide who wins under load (the worker scans before it locks).
        let leader = iteration % OP_COUNT;
        let (lead_done, lead_watch) = watch::channel(false);
        let mut lead_done = Some(ReleaseOnDrop(lead_done));
        let barrier = Arc::new(Barrier::new(OP_COUNT - 1));
        let handles: Vec<_> = OPS
            .iter()
            .copied()
            .enumerate()
            .map(|(index, op)| {
                let barrier = barrier.clone();
                let services = env.services().clone();
                let store = env.store().clone();
                let race = race.clone();
                let release = if index == leader {
                    lead_done.take()
                } else {
                    None
                };
                let mut lead_watch = lead_watch.clone();
                let head_start = HEAD_START_STEP * ((index + iteration) % OP_COUNT) as u32;
                tokio::spawn(async move {
                    if release.is_none() {
                        lead_watch.wait_for(|done| *done).await.ok();
                        barrier.wait().await;
                        tokio::time::sleep(head_start).await;
                    }
                    let outcome =
                        tokio::time::timeout(OP_TIMEOUT, run(op, services, store, race)).await;
                    drop(release);
                    (op, outcome)
                })
            })
            .collect();

        let mut worker_ran = false;
        let mut checkout_completed = false;
        for (index, handle) in handles.into_iter().enumerate() {
            let (op, outcome) = handle.await.expect("operation task panicked");
            let Ok(result) = outcome else {
                panic!(
                    "iteration {iteration}: {op:?} still running after {OP_TIMEOUT:?}: a lock-order hang"
                );
            };
            match result {
                Ok(done) => {
                    match op {
                        Op::Worker => worker_ran = true,
                        Op::Checkout => checkout_completed = done,
                        _ => {}
                    }
                    if done && op != Op::Worker {
                        succeeded[index] += 1;
                    }
                }
                Err(error) => {
                    if let Some(failure) = lock_failure(&error) {
                        panic!("iteration {iteration}: {op:?} failed with `{failure}`: {error}");
                    }
                    println!(
                        "iteration {iteration}: {op:?} refused: {}",
                        error.lines().take(12).collect::<Vec<_>>().join(" | ")
                    );
                }
            }
        }

        // The worker records a failed renewal on the subscription instead of returning it.
        let after = env.get_subscription(race.subscription_id).await;
        if let Some(error) = &after.last_error
            && let Some(failure) = lock_failure(error)
        {
            panic!("iteration {iteration}: the worker's renewal failed with `{failure}`: {error}");
        }
        if worker_ran && after.cycle_index > race.cycle_index_before {
            succeeded[OPS.iter().position(|op| *op == Op::Worker).unwrap()] += 1;
        }

        raced.push(Raced {
            customer_id: race.customer_id,
            subscription_id: race.subscription_id,
            checkout_completed,
        });
    }

    for (op, count) in OPS.iter().zip(succeeded) {
        println!("{op:?}: succeeded in {count}/{ITERATIONS} iterations");
    }
    for (op, count) in OPS.iter().zip(succeeded) {
        assert!(
            count > 0,
            "{op:?} never succeeded in {ITERATIONS} iterations: the race exercised nothing of it"
        );
    }

    for outcome in &raced {
        assert_ledger_balances(env.store(), TENANT_ID, outcome.customer_id).await;

        for invoice in env.get_customer_invoices(outcome.customer_id).await {
            assert!(
                invoice.amount_due >= 0,
                "invoice {} of customer {} owes {}",
                invoice.id,
                outcome.customer_id,
                invoice.amount_due
            );
        }

        // The settlement, credit-note, recall and main subscriptions, plus a completed checkout's.
        let expected = 4 + i64::from(outcome.checkout_completed);
        assert_eq!(
            subscriptions_of(&env, outcome.customer_id).await,
            expected,
            "customer {} has a subscription per completed checkout and no other",
            outcome.customer_id
        );

        let sub = env.get_subscription(outcome.subscription_id).await;
        assert_ne!(
            sub.status,
            SubscriptionStatusEnum::Errored,
            "subscription {} errored: {:?}",
            outcome.subscription_id,
            sub.last_error
        );
        assert_eq!(
            sub.error_count, 0,
            "subscription {} failed a renewal: {:?}",
            outcome.subscription_id, sub.last_error
        );
    }
}

async fn run(op: Op, services: Services, store: Store, race: Arc<Race>) -> Result<bool, String> {
    match op {
        Op::Finalize => services
            .finalize_invoice(Actor::System, race.draft_invoice_id, TENANT_ID)
            .await
            .map(|_| true)
            .map_err(chain),
        Op::PlanChange => services
            .apply_plan_change_immediate_at(
                Actor::System,
                race.subscription_id,
                TENANT_ID,
                PLAN_VERSION_PRO_ID,
                vec![],
                race.change_date,
            )
            .await
            .map(|_| true)
            .map_err(chain),
        Op::Seats => services
            .update_subscription_slots_for_test(
                TENANT_ID,
                race.subscription_id,
                COMP_STARTER_SEATS_ID,
                2,
                SlotUpgradeBillingMode::Optimistic,
                Some(race.seats_at),
            )
            .await
            .map(|_| true)
            .map_err(chain),
        Op::Settlement => settle(&services, &store, &race).await.map(|()| true),
        Op::Worker => services
            .get_and_process_cycle_transitions()
            .await
            .map(|_| true)
            .map_err(chain),
        Op::CreditNoteVoid => store
            .void_credit_note(Actor::System, TENANT_ID, race.credit_note_id)
            .await
            .map(|_| true)
            .map_err(chain),
        Op::Checkout => services
            .complete_checkout(
                TENANT_ID,
                race.checkout_session_id,
                race.payment_method_id,
                race.checkout_amount,
                CURRENCY.to_string(),
                None,
            )
            .await
            .map(|result| matches!(result, CheckoutCompletionResult::Completed { .. }))
            .map_err(chain),
        Op::Chargeback => services
            .reverse_manual_payment(
                Actor::System,
                TENANT_ID,
                ReverseManualPaymentCommand {
                    transaction_id: race.wire_id,
                    kind: ReversalKind::DebtorRecall,
                    reason: Some("Lock stress recall".to_string()),
                    reversed_at: Utc::now().naive_utc(),
                },
            )
            .await
            .map(|_| true)
            .map_err(chain),
    }
}

// The webhook's outbox event is what triggers the settlement in production.
async fn settle(services: &Services, store: &Store, race: &Race) -> Result<(), String> {
    let connector = mock_connector();
    let connector_impl = initialize_payment_connector(&connector).map_err(chain)?;
    let event = connector_impl
        .parse_event(&race.webhook_payload, &HeaderMap::new())
        .map_err(chain)?
        .ok_or_else(|| "the webhook produced no event".to_string())?;
    handle_normalized_event(
        event,
        &connector,
        connector_impl.as_ref(),
        store.clone(),
        services,
    )
    .await
    .map_err(chain)?;
    let settled = store
        .last_settled_payment_tx_by_invoice_id(TENANT_ID, race.settled_invoice_id)
        .await
        .map_err(chain)?
        .ok_or_else(|| "the webhook settled no payment".to_string())?;
    services
        .on_payment_transaction_settled(settled.into())
        .await
        .map_err(chain)
}

fn chain(error: impl std::fmt::Debug) -> String {
    format!("{error:?}")
}

fn lock_failure(error: &str) -> Option<&'static str> {
    LOCK_FAILURES
        .into_iter()
        .find(|failure| error.contains(failure))
}

// Every step is asserted, so a failure inside the race is only ever a race outcome.
async fn prepare(env: &TestEnv, iteration: usize) -> Race {
    let today = Utc::now().date_naive();
    let customer_id = env
        .store()
        .insert_customer(Actor::System, customer(iteration), TENANT_ID)
        .await
        .expect("create the customer")
        .id;
    let connection_id = payment::get_or_create_customer_connection(
        env.pool(),
        customer_id,
        CustomerConnectionId::new(),
        MOCK_CONNECTOR_ID,
        &format!("mock_cus_lock_stress_{iteration}"),
    )
    .await;
    let payment_method_id = CustomerPaymentMethodId::new();
    payment::create_customer_payment_method(
        env.pool(),
        customer_id,
        connection_id,
        payment_method_id,
        &format!("mock_pm_lock_stress_{iteration}"),
    )
    .await;
    payment::set_customer_default_payment_method(env.pool(), customer_id, payment_method_id).await;

    // An unpaid invoice with a charge the provider confirms later, by webhook.
    let settled_invoice = only_invoice(env, card_subscription(env, customer_id, today).await).await;
    assert_eq!(settled_invoice.status, InvoiceStatusEnum::Finalized);
    assert!(
        settled_invoice.amount_due > 0,
        "the invoice has a balance to charge"
    );
    env.set_mock_charge_behavior(MockChargeBehavior::Pending)
        .await;
    let (pending, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            settled_invoice.id,
            payment_method_id,
            false,
            None,
        )
        .await
        .expect("the charge is accepted");
    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    assert_eq!(pending.status, PaymentStatusEnum::Pending);
    let external_id = pending
        .provider_transaction_id
        .clone()
        .expect("a pending charge records its provider id");
    let webhook_payload = format!(
        r#"{{"id":"evt_ok_{tx}","kind":"payment_succeeded","transaction_id":"{tx}","external_id":"{external_id}","amount":{amount},"currency":"{currency}"}}"#,
        tx = pending.id.as_base62(),
        amount = pending.amount,
        currency = pending.currency,
    )
    .into_bytes();

    // An unpaid invoice whose debt-cancelling note the void reopens; a credit-to-balance note
    // would leave customer-wide credit that the checkout's first invoice draws, changing its price.
    let credited_invoice =
        only_invoice(env, card_subscription(env, customer_id, today).await).await;
    assert_eq!(credited_invoice.status, InvoiceStatusEnum::Finalized);
    assert!(credited_invoice.amount_due > 0);
    let note = env
        .store()
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: credited_invoice.id,
                line_items: vec![],
                reason: Some("Lock stress".to_string()),
                memo: None,
                credit_type: CreditType::DebtCancellation,
                custom_properties: None,
            },
        )
        .await
        .expect("issue the credit note");
    assert_eq!(
        note.debt_cancelled_amount_cents,
        credited_invoice.amount_due
    );

    // A paid invoice whose wire the payer recalls during the race.
    let wired_invoice = only_invoice(env, card_subscription(env, customer_id, today).await).await;
    assert_eq!(wired_invoice.status, InvoiceStatusEnum::Finalized);
    let wire = env
        .services()
        .add_manual_payment_transaction(
            Actor::System,
            TENANT_ID,
            wired_invoice.id,
            rust_decimal::Decimal::new(wired_invoice.amount_due, 2),
            Utc::now().fixed_offset(),
            Some(format!("wire-lock-stress-{iteration}")),
        )
        .await
        .expect("record the wire");
    env.run_outbox_and_orchestration().await;

    // Started a period ago: its period ends today, so the worker finds it due.
    let start = today
        .checked_sub_months(Months::new(1))
        .expect("a month ago");
    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .no_auto_advance()
        .create(env.services())
        .await;
    let sub = env.get_subscription(subscription_id).await;
    let period_end = sub.current_period_end.expect("a billing period");
    assert!(period_end <= today, "the subscription is due for renewal");
    let draft = only_invoice(env, subscription_id).await;
    assert_eq!(draft.status, InvoiceStatusEnum::Draft);
    assert!(draft.total > 0, "the period draft bills the plan");
    // Dated inside the period so both move money; the seats come first so a seat upgrade landing
    // after the change is refused rather than dated before it.
    let change_date = sub.current_period_start + chrono::Duration::days(10);
    let seats_at = (sub.current_period_start + chrono::Duration::days(8)).and_time(NaiveTime::MIN);
    assert!(change_date < period_end);

    // Scoped to the subscription, so its draft and the change move the ledger while the checkout's
    // invoice, priced without credit, is charged what the session showed.
    fund_subscription_grant(
        env.store(),
        TENANT_ID,
        customer_id,
        subscription_id,
        Grant {
            application: CreditApplication::Invoice,
            amount_minor: 50_000,
            priority: DEFAULT_PRIORITY,
            expires_at: None,
        },
    )
    .await;

    let session = env
        .store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            billing_start_date: Some(today),
            billing_day_anchor: Some(today.day() as i16),
            net_terms: None,
            trial_duration_days: None,
            end_date: None,
            auto_advance_invoices: true,
            charge_automatically: true,
            invoice_memo: None,
            invoice_threshold: None,
            purchase_order: None,
            payment_methods_config: None,
            components: None,
            add_ons: None,
            coupon_code: None,
            coupon_ids: vec![],
            expires_in_hours: Some(24),
            metadata: None,
            checkout_type: CheckoutType::SelfServe,
            subscription_id: None,
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("create the checkout session");

    Race {
        customer_id,
        payment_method_id,
        subscription_id,
        cycle_index_before: sub.cycle_index,
        draft_invoice_id: draft.id,
        change_date,
        seats_at,
        settled_invoice_id: settled_invoice.id,
        webhook_payload,
        credit_note_id: note.id,
        checkout_session_id: session.id,
        // The same plan over a full period: what the checkout bills.
        checkout_amount: u64::try_from(settled_invoice.total).expect("a positive total"),
        wire_id: wire.id,
    }
}

// Mirrors the seeded Uber customer, so its invoices bill and tax the same way.
fn customer(iteration: usize) -> CustomerNew {
    CustomerNew {
        name: format!("Lock stress {iteration}"),
        customer_type: CustomerType::Company,
        first_name: None,
        last_name: None,
        alias: None,
        billing_email: None,
        invoicing_emails: vec![],
        phone: None,
        currency: CURRENCY.to_string(),
        billing_address: None,
        shipping_address: None,
        invoicing_entity_id: Some(INVOICING_ENTITY_ID),
        force_created_date: None,
        tax_status: CustomerTaxStatus::Taxable,
        exemption_reason: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: vec![],
        connected_account_id: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
    }
}

async fn card_subscription(
    env: &TestEnv,
    customer_id: CustomerId,
    start: NaiveDate,
) -> SubscriptionId {
    subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(start)
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await
}

async fn only_invoice(env: &TestEnv, subscription_id: SubscriptionId) -> Invoice {
    let mut invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1, "one invoice for the first period");
    invoices.remove(0)
}

async fn subscriptions_of(env: &TestEnv, customer_id: CustomerId) -> i64 {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::subscription::dsl as s;

    let mut conn = env.conn().await;
    s::subscription
        .filter(s::customer_id.eq(customer_id))
        .count()
        .get_result(&mut conn)
        .await
        .expect("count the customer's subscriptions")
}

// The dispatcher reads only the tenant and provider of the connector `seed_payments` stores.
fn mock_connector() -> Connector {
    Connector {
        status: ConnectorStatusEnum::Active,
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
