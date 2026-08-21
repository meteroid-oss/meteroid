//! GoCardless hosted-checkout flow: ONE combined mandate+payment Billing Request
//! collects the first payment together with the mandate.
//!
//! Lifecycle under test:
//!   1. `initiate_hosted_checkout` → Pending checkout tx (invoice_id NULL,
//!      checkout_session_id set, RedirectToUrl next_action), session AwaitingPayment.
//!      Re-invocation reuses the SAME tx/redirect (one attempt, no second BR).
//!   2. `billing_requests.fulfilled` → `on_hosted_checkout_fulfilled` (the exact
//!      seam the webhook dispatcher calls after fetching the BR snapshot from the
//!      GoCardless API — which is why the test drives it directly): mandate
//!      attached as a payment method, subscription created/activated in-flight,
//!      first invoice Finalized + Processing, the still-Pending tx LINKED to it.
//!   3. `payments.confirmed` (real GoCardless fixture through the real GC parser
//!      and production dispatcher) → tx Settled, invoice Processing → Paid.
//!   4. `payments.failed` after in-flight activation → tx Failed, invoice reopens
//!      as Errored (collectible, on the dunning ladder), subscription stays Active.
//!
//! Every webhook leg is re-delivered to prove redelivery idempotency.

use chrono::NaiveDate;
use http::HeaderMap;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};

use meteroid::api_rest::webhooks::event_handler::handle_normalized_event;
use meteroid_store::adapters::payment::events::NormalizedEventKind;
use meteroid_store::adapters::payment::initialize_payment_connector;
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::checkout_sessions::{
    CheckoutCompletionResult, CheckoutSession, CheckoutSessionStatus, CheckoutType,
};
use meteroid_store::domain::connectors::{Connector, GocardlessPublicData, ProviderData};
use meteroid_store::domain::enums::{
    ConnectorProviderEnum, ConnectorTypeEnum, InvoicePaymentStatus, InvoiceStatusEnum,
};
use meteroid_store::domain::payment_transactions::PaymentNextAction;
use meteroid_store::domain::{CreateCheckoutSession, PaymentTransaction};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

use common_domain::ids::{
    BaseId, CheckoutSessionId, ConnectorId, PaymentTransactionId, SubscriptionId,
};

/// The provider payment id GoCardless creates for the combined BR's
/// payment_request — shared between the test and the webhook fixtures
/// (`links.payment` / `links.payment_request_payment`).
const HOSTED_PAYMENT_ID: &str = "PM_HOSTED_CK_001";

/// A GoCardless connector domain object. Only identity (tenant/provider) and the
/// parser matter here: fixtures go through the REAL GoCardless `parse_event`, and
/// the dispatcher resolves transactions within `connector.tenant_id`. No row is
/// needed in the DB and no API credentials are used (parse is offline).
fn gocardless_connector() -> Connector {
    Connector {
        id: ConnectorId::new(),
        created_at: chrono::Utc::now().naive_utc(),
        tenant_id: TENANT_ID,
        alias: "gocardless-hosted-checkout-test".to_string(),
        connector_type: ConnectorTypeEnum::PaymentProvider,
        provider: ConnectorProviderEnum::Gocardless,
        data: Some(ProviderData::Gocardless(GocardlessPublicData {
            creditor_id: Some("CR000TEST0001".to_string()),
            environment: "sandbox".to_string(),
        })),
        sensitive: None,
    }
}

/// Parse a raw GoCardless webhook body with the real GC adapter and run it
/// through the production dispatcher, then drain the outbox so settlement /
/// failure orchestration runs — the same path the HTTP webhook route drives
/// (minus signature verification, unit-tested per adapter).
async fn deliver_gocardless_webhook(env: &TestEnv, payload: &str) {
    let connector = gocardless_connector();
    let connector_impl =
        initialize_payment_connector(&connector).expect("gocardless connector init");

    let event = connector_impl
        .parse_event(&connector, payload.as_bytes(), &HeaderMap::new())
        .expect("parse_event ok")
        .expect("an event was produced");

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

/// `payments.confirmed` for the hosted first payment, carrying our ids in
/// `resource_metadata` exactly as GoCardless echoes the payment_request metadata.
fn confirmed_payload(tx_id: PaymentTransactionId) -> String {
    include_str!("../fixtures/webhooks/gocardless/hosted_checkout_payment_confirmed.json")
        .replace("{{TENANT_ID}}", &TENANT_ID.as_base62())
        .replace("{{TRANSACTION_ID}}", &tx_id.as_base62())
}

/// `payments.failed` (insufficient funds) for the hosted first payment.
fn failed_payload(tx_id: PaymentTransactionId) -> String {
    include_str!("../fixtures/webhooks/gocardless/hosted_checkout_payment_failed.json")
        .replace("{{TENANT_ID}}", &TENANT_ID.as_base62())
        .replace("{{TRANSACTION_ID}}", &tx_id.as_base62())
}

/// Raw payment_transaction row (the domain object ghosts `next_action` to None,
/// so persistence assertions read the row directly).
async fn tx_row(
    env: &TestEnv,
    tx_id: PaymentTransactionId,
) -> diesel_models::payments::PaymentTransactionRow {
    let mut conn = env.conn().await;
    diesel_models::payments::PaymentTransactionRow::get_by_id(&mut conn, tx_id, TENANT_ID)
        .await
        .expect("get payment transaction row")
}

async fn session(env: &TestEnv, id: CheckoutSessionId) -> CheckoutSession {
    env.store()
        .get_checkout_session(TENANT_ID, id)
        .await
        .expect("get checkout session")
}

/// A SelfServe checkout session for Uber on the LeetCode plan ($35/mo, EUR).
async fn create_selfserve_session(env: &TestEnv) -> CheckoutSession {
    env.store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            billing_start_date: Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()),
            billing_day_anchor: Some(1),
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
        })
        .await
        .expect("create checkout session")
}

/// Call `initiate_hosted_checkout` and unwrap the AwaitingPayment result into
/// `(transaction, redirect_url)`, asserting the redirect is a RedirectToUrl.
async fn initiate(
    env: &TestEnv,
    session_id: CheckoutSessionId,
    amount_minor: i64,
) -> (PaymentTransaction, String) {
    let result = env
        .services()
        .initiate_hosted_checkout(
            TENANT_ID,
            session_id,
            CUST_UBER_CONNECTION_ID,
            amount_minor,
            "EUR".to_string(),
            None, // coupon_code
            None, // return_url
        )
        .await
        .expect("initiate_hosted_checkout succeeds");

    match result {
        CheckoutCompletionResult::AwaitingPayment {
            transaction,
            next_action,
        } => {
            let url = match next_action {
                Some(PaymentNextAction::RedirectToUrl { url }) => url,
                other => panic!("hosted checkout must return a RedirectToUrl, got {other:?}"),
            };
            (transaction, url)
        }
        CheckoutCompletionResult::Completed { .. } => {
            panic!("hosted checkout is asynchronous; it must never complete inline")
        }
    }
}

/// Assert the initiated state: ONE Pending checkout tx anchoring the hosted
/// attempt (invoice_id NULL, session id set, redirect persisted), session
/// AwaitingPayment — and that a re-click reuses that same attempt instead of
/// minting a second Billing Request / transaction.
async fn assert_initiated_and_idempotent(
    env: &TestEnv,
    session_id: CheckoutSessionId,
    tx: &PaymentTransaction,
    url: &str,
    amount_minor: i64,
) {
    assert_eq!(tx.status, PaymentStatusEnum::Pending);
    assert_eq!(tx.amount, amount_minor);
    assert_eq!(
        tx.invoice_id, None,
        "no invoice exists yet — materialization happens at fulfillment"
    );
    assert_eq!(tx.checkout_session_id, Some(session_id));
    assert_eq!(
        tx.provider_transaction_id, None,
        "the BR id must not be stored as a payment id (reconciliation would probe it)"
    );
    assert!(!url.is_empty(), "redirect URL drives the hosted flow");

    assert_eq!(
        session(env, session_id).await.status,
        CheckoutSessionStatus::AwaitingPayment
    );

    // The redirect is persisted on the row so a re-click can re-hydrate it.
    let row = tx_row(env, tx.id).await;
    assert!(
        row.next_action.is_some(),
        "next_action must be persisted on the pending checkout transaction"
    );

    // Re-invocation (double click / back button): SAME transaction, SAME
    // redirect, still exactly one attempt — never a second Billing Request.
    let (tx2, url2) = initiate(env, session_id, amount_minor).await;
    assert_eq!(tx2.id, tx.id, "re-initiate returns the SAME transaction");
    assert_eq!(url2, url, "re-initiate re-hydrates the SAME redirect URL");
    assert_eq!(
        env.get_transactions_by_checkout_session(session_id)
            .await
            .len(),
        1,
        "still exactly one checkout transaction after re-initiate"
    );
}

/// Drive `billing_requests.fulfilled` for the combined BR: in production the
/// dispatcher fetches the BR from the GoCardless API, upserts the mandate as a
/// payment method and calls this seam with the created payment id
/// (`links.payment_request_payment`). The API fetch can't run offline, so the
/// test seeds the SEPA method and enters at the seam.
async fn fulfill(env: &TestEnv, session_id: CheckoutSessionId) {
    env.services()
        .on_hosted_checkout_fulfilled(
            TENANT_ID,
            session_id,
            CUST_UBER_SEPA_METHOD_ID,
            Some(HOSTED_PAYMENT_ID.to_string()),
            None,
        )
        .await
        .expect("on_hosted_checkout_fulfilled succeeds");
}

/// Assert the in-flight materialized state after `billing_requests.fulfilled`:
/// subscription ACTIVE, first invoice Finalized+Processing with its FULL
/// amount_due (funds not landed yet), the pre-created tx linked and still
/// Pending, session Completed. Returns `(invoice_id, total)`.
async fn assert_materialized_in_flight(
    env: &TestEnv,
    session_id: CheckoutSessionId,
    sub_id: SubscriptionId,
    tx_id: PaymentTransactionId,
    expected_total: i64,
) -> common_domain::ids::InvoiceId {
    // Session completed and bound to the subscription.
    let sess = session(env, session_id).await;
    assert_eq!(sess.status, CheckoutSessionStatus::Completed);
    assert_eq!(sess.subscription_id, Some(sub_id));

    // Subscription activated in-flight.
    let sub = env.get_subscription(sub_id).await;
    assert!(
        sub.activated_at.is_some(),
        "fulfillment must activate the subscription (activated_at set)"
    );
    sub.assert().is_active().has_pending_checkout(false);

    // First invoice: contractual document finalized, payment in flight.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_processing()
        .has_total(expected_total)
        .has_amount_due(expected_total); // amount_due only drops on real settlement

    // The pre-created tx is linked to that invoice, still Pending, now carrying
    // the provider payment id and the attached method; the redirect is consumed.
    let row = tx_row(env, tx_id).await;
    assert_eq!(row.invoice_id, Some(invoices[0].id));
    assert_eq!(
        row.status,
        diesel_models::enums::PaymentStatusEnum::Pending,
        "fulfillment accepts the payment; only payments.confirmed settles it"
    );
    assert_eq!(
        row.provider_transaction_id.as_deref(),
        Some(HOSTED_PAYMENT_ID),
        "tx must be re-keyed to the created payment (links.payment_request_payment)"
    );
    assert_eq!(row.payment_method_id, Some(CUST_UBER_SEPA_METHOD_ID));
    assert!(
        row.next_action.is_none(),
        "the redirect must be cleared once the hosted flow is fulfilled"
    );

    invoices[0].id
}

/// Settle via `payments.confirmed` (twice — redelivery must be a no-op) and
/// assert invoice Paid with zero due and the tx Settled.
async fn confirm_and_assert_paid(
    env: &TestEnv,
    sub_id: SubscriptionId,
    tx_id: PaymentTransactionId,
) {
    deliver_gocardless_webhook(env, &confirmed_payload(tx_id)).await;

    let row = tx_row(env, tx_id).await;
    assert_eq!(row.status, diesel_models::enums::PaymentStatusEnum::Settled);
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);

    // Redelivered payments.confirmed: still Paid, amount_due stays 0.
    deliver_gocardless_webhook(env, &confirmed_payload(tx_id)).await;
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().invoice_at(0).is_finalized_paid();
    invoices.assert().invoice_at(0).has_amount_due(0);
    assert_eq!(
        tx_row(env, tx_id).await.status,
        diesel_models::enums::PaymentStatusEnum::Settled
    );
}

// =============================================================================
// SelfServe: initiate → fulfilled (subscription created in-flight) → confirmed
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_hosted_checkout_selfserve_lifecycle(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_uber_sepa_payment_method().await;

    let sess = create_selfserve_session(&env).await;

    // 1. Initiate: one Pending anchored attempt + redirect; re-click reuses it.
    let (tx, url) = initiate(&env, sess.id, 3500).await;
    assert_initiated_and_idempotent(&env, sess.id, &tx, &url, 3500).await;

    // 2. Fulfilled: subscription + invoice materialized in-flight.
    fulfill(&env, sess.id).await;
    let sub_id = session(&env, sess.id)
        .await
        .subscription_id
        .expect("SelfServe fulfillment creates and binds the subscription");
    assert_materialized_in_flight(&env, sess.id, sub_id, tx.id, 3500).await;

    // Draining the pipeline (PDF, auto-charge orchestration) must not mint a
    // second charge: the linked Pending tx owns the invoice.
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        env.get_transactions_by_checkout_session(sess.id)
            .await
            .len(),
        1,
        "orchestration must not auto-charge an invoice owned by an in-flight payment"
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_processing();

    // Redelivered fulfilled webhook: a no-op (tx already linked, session done).
    fulfill(&env, sess.id).await;
    env.get_invoices(sub_id).await.assert().has_count(1);
    assert_eq!(
        env.get_transactions_by_checkout_session(sess.id)
            .await
            .len(),
        1
    );

    // 3. payments.confirmed settles: Processing → Paid (redelivery-safe).
    confirm_and_assert_paid(&env, sub_id, tx.id).await;

    // Late fulfilled redelivery after settlement is still a no-op.
    fulfill(&env, sess.id).await;
    env.get_invoices(sub_id).await.assert().has_count(1);
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .is_finalized_paid();
}

// =============================================================================
// SubscriptionActivation: existing OnCheckout subscription activates in-flight
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_hosted_checkout_activation_lifecycle(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_uber_sepa_payment_method().await;

    // An OnCheckout subscription auto-creates a SubscriptionActivation session.
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    env.get_subscription(sub_id)
        .await
        .assert()
        .is_pending_activation()
        .has_pending_checkout(true);

    let sess = env
        .store()
        .get_checkout_session_by_subscription(TENANT_ID, sub_id)
        .await
        .expect("activation checkout session auto-created");
    assert_eq!(sess.checkout_type, CheckoutType::SubscriptionActivation);

    let (tx, url) = initiate(&env, sess.id, 3500).await;
    assert_initiated_and_idempotent(&env, sess.id, &tx, &url, 3500).await;

    // Not activated while the customer is still in the hosted flow.
    env.get_subscription(sub_id)
        .await
        .assert()
        .is_pending_activation();
    env.get_invoices(sub_id).await.assert().assert_empty();

    fulfill(&env, sess.id).await;
    assert_materialized_in_flight(&env, sess.id, sub_id, tx.id, 3500).await;

    // Redelivered fulfilled: no double bill / re-activation.
    fulfill(&env, sess.id).await;
    env.get_invoices(sub_id).await.assert().has_count(1);
    assert_eq!(
        env.get_transactions_by_checkout_session(sess.id)
            .await
            .len(),
        1
    );

    confirm_and_assert_paid(&env, sub_id, tx.id).await;
}

// =============================================================================
// payments.failed after in-flight activation: the invoice reopens for dunning,
// the subscription keeps its access (dunning-policy decision), tx is Failed.
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_hosted_checkout_first_payment_failure_reopens_invoice(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_uber_sepa_payment_method().await;

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .start_date(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;
    let sess = env
        .store()
        .get_checkout_session_by_subscription(TENANT_ID, sub_id)
        .await
        .expect("activation checkout session auto-created");

    let (tx, _) = initiate(&env, sess.id, 3500).await;
    fulfill(&env, sess.id).await;
    let invoice_id = assert_materialized_in_flight(&env, sess.id, sub_id, tx.id, 3500).await;

    // Drain the finalize/PDF pipeline NOW, while the Pending tx still fences the
    // invoice from auto-charge — production timing: orchestration runs promptly,
    // the bank's failure webhook arrives days later.
    env.run_outbox_and_orchestration().await;
    assert_eq!(
        env.get_transactions_by_checkout_session(sess.id)
            .await
            .len(),
        1,
        "the in-flight payment must fence the invoice from auto-charge"
    );

    // The bank bounces the first debit.
    deliver_gocardless_webhook(&env, &failed_payload(tx.id)).await;
    // Failure orchestration (Errored + dunning) runs off the outbox event.
    env.run_outbox_and_orchestration().await;

    let row = tx_row(&env, tx.id).await;
    assert_eq!(row.status, diesel_models::enums::PaymentStatusEnum::Failed);

    // No money moved: the invoice is a collectible document again — visibly
    // errored, full amount due, on the dunning ladder. Never Processing/Paid.
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_status(InvoiceStatusEnum::Finalized)
        .has_payment_status(InvoicePaymentStatus::Errored)
        .has_amount_due(3500);

    let retries = env.pending_payment_retries(sub_id, invoice_id).await;
    assert_eq!(
        retries, 1,
        "the failed first collection schedules exactly one dunning retry"
    );

    // Access is not revoked by the failed collection in this iteration.
    env.get_subscription(sub_id).await.assert().is_active();

    // Redelivered failure: same terminal state, no stacked retries.
    deliver_gocardless_webhook(&env, &failed_payload(tx.id)).await;
    env.run_outbox_and_orchestration().await;
    let invoices = env.get_invoices(sub_id).await;
    invoices
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Errored)
        .has_amount_due(3500);
    assert_eq!(env.pending_payment_retries(sub_id, invoice_id).await, 1);
}

// =============================================================================
// PlanChange (free trial → no-trial plan): the hosted first payment upgrades the
// plan in-flight; payments.confirmed closes the new plan's first invoice.
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_hosted_checkout_plan_change_from_free_trial(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_uber_sepa_payment_method().await;

    // Trial must still be running at fulfillment time: the deferred (webhook)
    // plan-change path recomputes at the CURRENT date, not the stored change_date.
    let today = chrono::Utc::now().date_naive();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // $49/mo, 14d free trial
        .start_date(today)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    env.get_subscription(sub_id)
        .await
        .assert()
        .is_trial_active();
    env.get_invoices(sub_id).await.assert().assert_empty();

    let customer_id = env.get_subscription(sub_id).await.customer_id;
    let sess = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID, // €29 + €10 = €39/mo, no trial
            customer_id,
            None,
            today,
        )
        .await
        .expect("create plan change checkout session");
    assert_eq!(sess.checkout_type, CheckoutType::PlanChange);

    let (tx, url) = initiate(&env, sess.id, 3900).await;
    assert_initiated_and_idempotent(&env, sess.id, &tx, &url, 3900).await;

    fulfill(&env, sess.id).await;

    // Free-trial upgrade: no proration adjustment — the new plan's first full
    // period is billed, Processing, linked to the pre-created tx.
    let sub = env.get_subscription(sub_id).await;
    assert_eq!(sub.plan_version_id, PLAN_VERSION_STARTER_ID);
    sub.assert().is_active();
    assert_eq!(
        session(&env, sess.id).await.status,
        CheckoutSessionStatus::Completed
    );

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .is_finalized_processing()
        .has_total(3900)
        .has_amount_due(3900);

    let row = tx_row(&env, tx.id).await;
    assert_eq!(row.invoice_id, Some(invoices[0].id));
    assert_eq!(row.status, diesel_models::enums::PaymentStatusEnum::Pending);

    // Redelivered fulfilled is a no-op on a completed session.
    fulfill(&env, sess.id).await;
    env.get_invoices(sub_id).await.assert().has_count(1);

    confirm_and_assert_paid(&env, sub_id, tx.id).await;
}

// =============================================================================
// Fixture shape: the combined mandate+payment `billing_requests.fulfilled` event
// normalizes to MandateSetupCompleted carrying the BR id — the trigger through
// which the production dispatcher fetches the BR (metadata + created payment)
// and reaches `on_hosted_checkout_fulfilled`.
// =============================================================================

#[rstest]
#[tokio::test]
async fn test_combined_billing_request_fulfilled_parses_to_mandate_setup_completed(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    drop(env); // parse-only check; the fixture must still be a valid GC envelope

    let payload = include_str!(
        "../fixtures/webhooks/gocardless/hosted_checkout_billing_request_fulfilled.json"
    )
    .replace("{{CUSTOMER_ID}}", &CUST_UBER_ID.as_base62())
    .replace("{{CONNECTION_ID}}", &CUST_UBER_CONNECTION_ID.as_base62())
    .replace(
        "{{CHECKOUT_SESSION_ID}}",
        &CheckoutSessionId::new().as_base62(),
    );

    let connector = gocardless_connector();
    let connector_impl =
        initialize_payment_connector(&connector).expect("gocardless connector init");

    let event = connector_impl
        .parse_event(&connector, payload.as_bytes(), &HeaderMap::new())
        .expect("parse ok")
        .expect("event produced");

    assert_eq!(event.provider_event_type, "billing_requests.fulfilled");
    match event.kind {
        NormalizedEventKind::MandateSetupCompleted { provider_intent_id } => {
            assert_eq!(
                provider_intent_id, "BRQ000HOSTED0001",
                "the BR id is the intent the dispatcher completes against the GC API"
            );
        }
        other => panic!("expected MandateSetupCompleted, got {other:?}"),
    }
}
