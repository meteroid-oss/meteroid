//! A collected payment is never silently dropped against a terminal invoice.
//!
//! Two checkout paths take the customer's money *before* the invoice that records it exists or is
//! known to be writable:
//!
//! - `bill_subscription_with_data_tx` in the `FinalizeAfterPayment` / `AlreadyPaid` modes, which
//!   arrive either with the charge already collected or with a currency/amount confirmation still
//!   to check;
//! - the free-trial plan-change checkout, which charges at the PSP and only afterwards creates the
//!   invoice to record the charge against.
//!
//! Both look up the period's invoice through `create_subscription_draft_invoice`, which returns any
//! existing non-void `Recurring` invoice for the date — including one a prior run already settled
//! (`Finalized` or `Closed`). Treating that as "nothing left to do" completes the checkout with the
//! money captured and no transaction recorded anywhere. These tests pin the opposite: the checkout
//! fails loudly, leaving a reconcilable state.
//!
//! The scheduler modes (`AwaitGracePeriodIfApplicable`, `Immediate`) carry no payment and keep
//! treating a terminal invoice as an idempotent no-op; that is the behaviour the guard was
//! originally written for and it is exercised by the renewal tests.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, SubscriptionAssertExt, TestEnv, subscription, test_env};
use meteroid_store::domain::CreateCheckoutSession;
use meteroid_store::domain::checkout_sessions::{CheckoutCompletionResult, CheckoutType};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

/// Build a second `SubscriptionActivation` checkout session for an already-activated subscription.
///
/// One is created automatically when a subscription is inserted with checkout activation; nothing
/// stops a second from being created through the API, and `complete_checkout`'s replay guard is
/// per-session, so a stale confirmation on the second session reaches the billing code with the
/// period already billed.
async fn create_activation_session(
    env: &TestEnv,
    subscription_id: common_domain::ids::SubscriptionId,
) -> meteroid_store::domain::checkout_sessions::CheckoutSession {
    let sub = env.get_subscription(subscription_id).await;

    env.store()
        .create_checkout_session(CreateCheckoutSession {
            tenant_id: TENANT_ID,
            customer_id: sub.customer_id,
            plan_version_id: sub.plan_version_id,
            billing_start_date: sub.billing_start_date,
            billing_day_anchor: Some(sub.billing_day_anchor),
            net_terms: Some(sub.net_terms),
            trial_duration_days: sub.trial_duration,
            end_date: sub.end_date,
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
            checkout_type: CheckoutType::SubscriptionActivation,
            subscription_id: Some(subscription_id),
            change_date: None,
            success_url: None,
            cancel_url: None,
        })
        .await
        .expect("Failed to create activation checkout session")
}

// =============================================================================
// FinalizeAfterPayment — bill_subscription_with_data_tx
// =============================================================================

/// A second activation confirmation, once the period is already billed and finalized, must be
/// rejected instead of reporting success without charging.
///
/// `FinalizeAfterPayment` is the mode that validates the client's currency/amount confirmation and
/// then charges. Skipping its arm because the invoice is terminal would return a "completed"
/// checkout that never ran either step — the caller then activates the subscription believing the
/// customer paid.
///
/// Reachable without fault injection: the replay guard in `complete_checkout` is scoped to a single
/// session, so a second activation session for the same subscription passes it.
#[rstest]
#[tokio::test]
async fn test_second_activation_checkout_on_billed_period_is_rejected(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID) // €39/mo, no trial
        .start_date(start_date)
        .on_checkout()
        .no_trial()
        .auto_charge()
        .create(env.services())
        .await;

    // First confirmation: the genuine one. Bills [Jan 1, Feb 1], charges €39, finalizes.
    let first_session = create_activation_session(&env, sub_id).await;
    let result = env
        .services()
        .complete_checkout(
            TENANT_ID,
            first_session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect("first activation confirmation should succeed");

    match result {
        CheckoutCompletionResult::TopUpCompleted { .. } => {
            panic!("subscription checkout returned a top-up")
        }
        CheckoutCompletionResult::Completed { .. } => {}
        CheckoutCompletionResult::AwaitingPayment { .. } => {
            panic!("Payment should not be pending for mock provider")
        }
    }

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices
        .assert()
        .invoice_at(0)
        .with_context("period billed by the first confirmation")
        .has_total(3900)
        // The transaction settles inline; `payment_status` flips to Paid only once the
        // PaymentTransactionSettled orchestration runs, which this test does not drive.
        .is_finalized_unpaid();
    let billed_invoice_id = invoices[0].id;

    // Second confirmation on a separate session, arriving after the period is finalized.
    let second_session = create_activation_session(&env, sub_id).await;
    let err = env
        .services()
        .complete_checkout(
            TENANT_ID,
            second_session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect_err("second confirmation must be rejected, not reported as completed");

    // Fail at the guard, not somewhere incidental in the setup.
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("terminal status"),
        "expected the terminal-invoice guard to reject the replay, got: {rendered}"
    );

    // The rejection left nothing behind: no second invoice, no second charge.
    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    assert_eq!(invoices[0].id, billed_invoice_id);

    let detailed = env.get_detailed_invoice(billed_invoice_id).await;
    assert_eq!(
        detailed.transactions.len(),
        1,
        "customer must be charged exactly once across both confirmations"
    );
    assert_eq!(detailed.transactions[0].amount, 3900);
}

// =============================================================================
// Free-trial plan change — ServicesEdge::complete_checkout_plan_change_tx
// =============================================================================

/// A plan change whose billed period is already settled must be rejected BEFORE the card is
/// charged. The free-trial path captures at the PSP and only afterwards creates the invoice to
/// record against; if the period turns out to be settled at that point, the rollback undoes the
/// session state but not the capture — and since the customer just sees a failed checkout, every
/// retry captures again. The pre-charge probe on the period's existing invoice stops the first
/// capture from ever happening; the post-charge guard remains as the last line of defense.
///
/// The terminal invoice is injected (see `inject_settled_period_invoice`) — no supported flow
/// settles a free trial's post-change period ahead of the change itself, which is exactly why the
/// guard is defensive.
#[rstest]
#[tokio::test]
async fn test_plan_change_settled_charge_with_terminal_invoice_is_rejected(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.seed_payments().await;

    let start_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let change_date = NaiveDate::from_ymd_opt(2024, 1, 8).unwrap();

    let sub_id = subscription()
        .plan_version(PLAN_VERSION_PAID_FREE_TRIAL_ID) // €49/mo, 14d free trial
        .start_date(start_date)
        .on_start()
        .trial_days(14)
        .auto_charge()
        .create(env.services())
        .await;

    let sub = env.get_subscription(sub_id).await;
    sub.assert().is_trial_active();

    // The plan change will move the period to start at `change_date`; pre-settle that period so the
    // post-charge invoice lookup returns a terminal invoice.
    let settled = env.inject_settled_period_invoice(sub_id, change_date).await;

    let session = env
        .services()
        .create_plan_change_checkout_session(
            TENANT_ID,
            sub_id,
            PLAN_VERSION_STARTER_ID, // €39/mo, no trial
            sub.customer_id,
            None,
            change_date,
        )
        .await
        .expect("create_plan_change_checkout_session failed");

    // Same amount the happy-path free-trial change charges (see plan_change_checkout.rs), so the
    // confirmation check passes and the flow reaches the pre-charge probe (never capturing).
    let err = env
        .services()
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            3900,
            "EUR".to_string(),
            None,
        )
        .await
        .expect_err("checkout must fail rather than complete with an unrecorded charge");

    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("refusing to charge"),
        "expected the PRE-charge probe to reject (never capturing at the PSP), got: {rendered}"
    );

    // Nothing was recorded against the settled period, and the plan change did not take effect.
    let detailed = env.get_detailed_invoice(settled.id).await;
    assert!(
        detailed.transactions.is_empty(),
        "no transaction may be attached to an already-settled period"
    );

    let sub = env.get_subscription(sub_id).await;
    assert_eq!(
        sub.plan_version_id, PLAN_VERSION_PAID_FREE_TRIAL_ID,
        "plan change must roll back with the failed checkout"
    );
    sub.assert().is_trial_active();
}
