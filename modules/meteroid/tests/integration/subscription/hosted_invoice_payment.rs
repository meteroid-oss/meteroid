//! Hosted invoice payment (Stancer in-flow capture) regression tests:
//! (1) the on-render SetupIntent is side-effect-free — no transaction, no
//! intent, no network; only the explicit pay action initiates; and (2) the
//! sweeper scan keeps Settled-with-marker (unmaterialized) rows visible.

use chrono::NaiveDate;
use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, subscription, test_env};

use common_domain::ids::{BaseId, PaymentTransactionId};
use diesel_models::payments::{PaymentTransactionRow, PaymentTransactionRowNew};
use meteroid_store::domain::PaymentStatusEnum;
use meteroid_store::domain::enums::ConnectorProviderEnum;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

/// A finalized, unpaid invoice for Uber (no auto-charge, so it stays payable).
async fn finalized_unpaid_invoice(env: &TestEnv) -> common_domain::ids::InvoiceId {
    let sub_id = subscription()
        .plan_version(PLAN_VERSION_1_LEETCODE_ID) // $35/month
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .no_auto_charge()
        .create(env.services())
        .await;

    let invoices = env.get_invoices(sub_id).await;
    invoices.assert().has_count(1);
    invoices.assert().invoice_at(0).is_finalized_unpaid();
    invoices[0].id
}

// The on-render SetupIntent for a Stancer card connection + invoice is a pure
// provider DESCRIPTOR — no transaction, no intent, no network.

#[rstest]
#[tokio::test]
async fn test_invoice_setup_intent_is_side_effect_free_for_stancer(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    // Stancer becomes the invoicing entity's card provider, with a pre-seeded
    // connection + saved card (no network involved).
    env.seed_stancer_payments().await;

    let invoice_id = finalized_unpaid_invoice(&env).await;

    // The payment panel fetches this on MOUNT. If it minted the capturing
    // intent, this call would hit the real Stancer API (and fail here); it
    // must instead return offline with a provider descriptor.
    let intent = env
        .services()
        .create_setup_intent(
            &TENANT_ID,
            &CUST_UBER_CONNECTION_STANCER_ID,
            Some(invoice_id),
            None,
        )
        .await
        .expect(
            "on-render setup intent must be side-effect-free (and offline) for Stancer+invoice",
        );

    assert_eq!(intent.provider, ConnectorProviderEnum::Stancer);
    assert_eq!(intent.connection_id, CUST_UBER_CONNECTION_STANCER_ID);
    assert!(
        intent.intent_id.is_empty() && intent.client_secret.is_empty(),
        "descriptor only: no provider intent may be minted on page render"
    );

    // No committed Pending transaction from the page view — repeated renders
    // included.
    let txs = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions");
    assert!(
        txs.is_empty(),
        "rendering the invoice page must not create a payment_transaction, got {txs:?}"
    );

    // Second render: still nothing.
    env.services()
        .create_setup_intent(
            &TENANT_ID,
            &CUST_UBER_CONNECTION_STANCER_ID,
            Some(invoice_id),
            None,
        )
        .await
        .expect("re-render is equally side-effect-free");
    assert!(
        env.store()
            .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
            .await
            .expect("list transactions")
            .is_empty()
    );

    // A saved-card customer can still pay immediately after the page rendered:
    // no phantom Pending transaction blocks the one-payment-at-a-time gate.
    let (tx, _) = env
        .services()
        .complete_invoice_payment(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            true,
            None,
        )
        .await
        .expect("saved-card payment must not be blocked by a mere page view");
    assert_eq!(tx.status, PaymentStatusEnum::Settled);
}

// Single-intent resume: while a hosted attempt is still Pending, re-initiating
// returns the SAME intent/redirect and mints nothing new; a Pending attempt
// WITHOUT the marker (off-session charge in flight) refuses a second initiation.

async fn insert_invoice_tx(
    env: &TestEnv,
    invoice_id: common_domain::ids::InvoiceId,
    marker: Option<&str>,
    next_action_url: Option<&str>,
) -> PaymentTransactionId {
    use meteroid_store::domain::payment_transactions::PaymentNextAction;

    let id = PaymentTransactionId::new();
    let row = PaymentTransactionRowNew {
        id,
        tenant_id: TENANT_ID,
        invoice_id: Some(invoice_id),
        provider_transaction_id: None,
        amount: 3500,
        currency: "EUR".to_string(),
        payment_method_id: None,
        status: diesel_models::enums::PaymentStatusEnum::Pending,
        payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: None,
        checkout_session_id: None,
        pending_plan_version_id: None,
        next_action: next_action_url.map(|url| {
            serde_json::to_value(PaymentNextAction::RedirectToUrl {
                url: url.to_string(),
            })
            .expect("serialize next_action")
        }),
        initiated_by_customer_id: Some(CUST_UBER_ID),
        pending_provider_intent_id: marker.map(str::to_string),
        pending_connection_id: marker.map(|_| CUST_UBER_CONNECTION_STANCER_ID),
    };
    let mut conn = env.conn().await;
    row.insert(&mut conn).await.expect("insert transaction row");
    id
}

#[rstest]
#[tokio::test]
async fn test_pending_hosted_attempt_resumes_same_intent(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_stancer_payments().await;

    let invoice_id = finalized_unpaid_invoice(&env).await;

    // An abandoned hosted attempt: Pending, marker + stored hosted redirect.
    insert_invoice_tx(
        &env,
        invoice_id,
        Some("pi_abandoned_hosted"),
        Some("https://payment.stancer.com/hosted/pi_abandoned_hosted"),
    )
    .await;

    // The portal exposes the resume affordance on this exact signal.
    let txs = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions");
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].transaction.status, PaymentStatusEnum::Pending);
    assert_eq!(
        txs[0].transaction.pending_connection_id,
        Some(CUST_UBER_CONNECTION_STANCER_ID)
    );

    // "Continue payment" re-runs the initiation: it must rehydrate the SAME
    // intent/redirect offline (a second capturable mint would hit the real
    // Stancer API and fail here).
    let intent = env
        .services()
        .initiate_hosted_invoice_payment(
            TENANT_ID,
            CUST_UBER_CONNECTION_STANCER_ID,
            invoice_id,
            None,
        )
        .await
        .expect("re-initiation while Pending must resume the stored intent");
    assert_eq!(intent.intent_id, "pi_abandoned_hosted");
    assert_eq!(
        intent.client_secret,
        "https://payment.stancer.com/hosted/pi_abandoned_hosted"
    );
    assert_eq!(intent.provider, ConnectorProviderEnum::Stancer);
    assert_eq!(intent.connection_id, CUST_UBER_CONNECTION_STANCER_ID);

    // No second transaction was created by the resume.
    let txs = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list transactions");
    assert_eq!(txs.len(), 1, "resume must not create a second transaction");
}

#[rstest]
#[tokio::test]
async fn test_pending_attempt_without_marker_refuses_reinitiation(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.seed_payments().await;
    env.seed_stancer_payments().await;

    let invoice_id = finalized_unpaid_invoice(&env).await;

    // An off-session charge in flight: Pending, NO hosted-intent marker.
    insert_invoice_tx(&env, invoice_id, None, None).await;

    let result = env
        .services()
        .initiate_hosted_invoice_payment(
            TENANT_ID,
            CUST_UBER_CONNECTION_STANCER_ID,
            invoice_id,
            None,
        )
        .await;
    assert!(
        result.is_err(),
        "a marker-less Pending attempt must refuse a hosted re-initiation"
    );
}

// The sweeper scan must include a Settled row that still carries its
// pending-intent marker (settled-but-unmaterialized), and only releasing the
// marker drops it out.

async fn insert_marked_tx(
    env: &TestEnv,
    status: diesel_models::enums::PaymentStatusEnum,
    marker: Option<&str>,
    checkout_session_id: common_domain::ids::CheckoutSessionId,
) -> PaymentTransactionId {
    let id = PaymentTransactionId::new();
    let row = PaymentTransactionRowNew {
        id,
        tenant_id: TENANT_ID,
        invoice_id: None,
        provider_transaction_id: None,
        amount: 3500,
        currency: "EUR".to_string(),
        payment_method_id: None,
        status,
        payment_type: diesel_models::enums::PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: None,
        checkout_session_id: Some(checkout_session_id),
        pending_plan_version_id: None,
        next_action: None,
        initiated_by_customer_id: None,
        pending_provider_intent_id: marker.map(str::to_string),
        pending_connection_id: marker.map(|_| CUST_UBER_CONNECTION_STANCER_ID),
    };
    let mut conn = env.conn().await;
    row.insert(&mut conn).await.expect("insert transaction row");
    id
}

#[rstest]
#[tokio::test]
async fn test_settled_row_with_marker_stays_sweepable_until_released(#[future] test_env: TestEnv) {
    use meteroid_store::domain::CreateCheckoutSession;
    use meteroid_store::domain::checkout_sessions::CheckoutType;
    use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;

    let env = test_env.await;
    env.seed_payments().await;
    env.seed_stancer_payments().await;

    let session = env
        .store()
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
        .expect("create checkout session");

    // Settled capture whose materialization never succeeded: marker retained.
    let settled_marked = insert_marked_tx(
        &env,
        diesel_models::enums::PaymentStatusEnum::Settled,
        Some("pi_settled_unmaterialized"),
        session.id,
    )
    .await;
    // Fully finished attempt: settled, marker already released.
    let settled_released = insert_marked_tx(
        &env,
        diesel_models::enums::PaymentStatusEnum::Settled,
        None,
        session.id,
    )
    .await;
    // Refunded rows are owned by reversal handling — never swept.
    let refunded_marked = insert_marked_tx(
        &env,
        diesel_models::enums::PaymentStatusEnum::Refunded,
        Some("pi_refunded"),
        session.id,
    )
    .await;
    // Classic in-flight attempt keeps being swept as before.
    let pending_marked = insert_marked_tx(
        &env,
        diesel_models::enums::PaymentStatusEnum::Pending,
        Some("pi_pending"),
        session.id,
    )
    .await;

    async fn list(env: &TestEnv) -> Vec<PaymentTransactionId> {
        let mut conn = env.conn().await;
        PaymentTransactionRow::list_sweepable_with_pending_intent(
            &mut conn,
            chrono::Utc::now() + chrono::Duration::hours(1),
            None,
            100,
        )
        .await
        .expect("sweep scan")
        .into_iter()
        .map(|r| r.id)
        .collect()
    }

    let scanned = list(&env).await;
    assert!(
        scanned.contains(&settled_marked),
        "a Settled row still carrying its marker (settled-but-unmaterialized) MUST stay sweepable"
    );
    assert!(scanned.contains(&pending_marked));
    assert!(
        !scanned.contains(&settled_released),
        "a finished attempt (marker released) must drop out of the scan"
    );
    assert!(
        !scanned.contains(&refunded_marked),
        "refunded rows belong to reversal handling, not the sweeper"
    );

    // A stale/mismatched intent id never clears the marker (supersede safety)…
    let mut conn = env.conn().await;
    let cleared = PaymentTransactionRow::clear_pending_intent_if_matches(
        &mut conn,
        TENANT_ID,
        settled_marked,
        "pi_some_other_intent",
    )
    .await
    .expect("guarded clear");
    assert_eq!(cleared, 0, "mismatched intent id must not clear the marker");
    drop(conn);
    assert!(list(&env).await.contains(&settled_marked));

    // …while releasing the real marker (what completion does once the
    // materialization succeeded) drops the row out of the scan for good.
    let mut conn = env.conn().await;
    let cleared = PaymentTransactionRow::clear_pending_intent_if_matches(
        &mut conn,
        TENANT_ID,
        settled_marked,
        "pi_settled_unmaterialized",
    )
    .await
    .expect("release marker");
    assert_eq!(cleared, 1);
    drop(conn);
    assert!(
        !list(&env).await.contains(&settled_marked),
        "once materialized (marker released) the row must stop being swept"
    );

    // The unconditional per-row clear (used inside the invoice settle
    // transaction) behaves identically.
    let mut conn = env.conn().await;
    let cleared = PaymentTransactionRow::clear_pending_intent(&mut conn, TENANT_ID, pending_marked)
        .await
        .expect("unconditional clear");
    assert_eq!(cleared, 1);
    drop(conn);
    assert!(!list(&env).await.contains(&pending_marked));
}
