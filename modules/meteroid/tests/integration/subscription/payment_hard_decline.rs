//! A hard decline (expired card, fraud, revoked mandate) stops the automatic retry ladder
//! for that instrument, and a new default payment method (or, for an expired card, refreshed
//! card details on the same method) revives it.

use rstest::rstest;

use crate::data::ids::*;
use crate::harness::{InvoicesAssertExt, TestEnv, test_env};

use chrono::Utc;
use common_domain::ids::{InvoiceId, SubscriptionId};
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{DeclineKind, InvoicePaymentStatus};
use meteroid_store::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
use meteroid_store::domain::{CustomerPatch, PaymentStatusEnum};
use meteroid_store::repositories::CustomersInterface;
use meteroid_store::repositories::SubscriptionInterface;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;

async fn hard_declined_invoice(env: &TestEnv) -> (SubscriptionId, InvoiceId) {
    let (sub_id, invoice_id) = env.unpaid_card_invoice().await;

    env.set_mock_charge_behavior(MockChargeBehavior::Expired)
        .await;
    let (tx, _) = env
        .services()
        .complete_invoice_payment_automatic(
            TENANT_ID,
            invoice_id,
            CUST_UBER_PAYMENT_METHOD_ID,
            None,
        )
        .await
        .expect("the decline is an outcome, not an error");
    assert_eq!(tx.status, PaymentStatusEnum::Failed);
    assert_eq!(tx.decline_kind, Some(DeclineKind::CardExpired));

    env.run_outbox_and_orchestration().await;
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Errored);

    (sub_id, invoice_id)
}

async fn replace_uber_card(env: &TestEnv) {
    crate::data::payment::create_customer_payment_method(
        env.pool(),
        CUST_UBER_ID,
        CUST_UBER_CONNECTION_ID,
        CUST_UBER_REPLACEMENT_CARD_ID,
        "mock_pm_uber_replacement_card",
    )
    .await;

    env.store()
        .patch_customer(
            Actor::System,
            TENANT_ID,
            CustomerPatch {
                id: CUST_UBER_ID,
                name: None,
                alias: None,
                billing_email: None,
                phone: None,
                currency: None,
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                current_payment_method_id: Some(Some(CUST_UBER_REPLACEMENT_CARD_ID)),
                invoicing_emails: None,
                tax_status: None,
                exemption_reason: None,
                custom_taxes: None,
                connected_account_id: None,
                custom_properties: None,
                preferred_locales: None,
                customer_type: None,
                first_name: None,
                last_name: None,
            },
        )
        .await
        .expect("patch customer");
}

#[rstest]
#[tokio::test]
async fn test_hard_decline_stops_retries_until_the_payment_method_changes(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let (sub_id, invoice_id) = hard_declined_invoice(&env).await;

    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        0,
        "retrying an expired card cannot succeed"
    );

    replace_uber_card(&env).await;
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        1,
        "a new payment method revives the retry"
    );

    // Setting the same method again must not stack a second retry.
    replace_uber_card(&env).await;
    assert_eq!(env.pending_payment_retries(sub_id, invoice_id).await, 1);

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.process_due_events().await;
    env.run_outbox_and_orchestration().await;

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Paid);

    let attempts = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions");
    assert!(attempts.iter().any(|t| {
        t.transaction.status == PaymentStatusEnum::Settled
            && t.transaction.payment_method_id == Some(CUST_UBER_REPLACEMENT_CARD_ID)
    }));
}

/// Stripe's card updater refreshes an expired card in place, keeping the method id.
async fn refresh_uber_card(env: &TestEnv, brand: &str, exp_year: i32) {
    env.store()
        .update_payment_method_card_details(
            TENANT_ID,
            "mock_pm_uber_card",
            Some(brand.to_string()),
            Some("4242".to_string()),
            Some(12),
            Some(exp_year),
        )
        .await
        .expect("refresh card details");
}

#[rstest]
#[tokio::test]
async fn test_refreshed_card_revives_an_expired_card_decline(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = hard_declined_invoice(&env).await;

    refresh_uber_card(&env, "mock_visa", 2030).await;
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        0,
        "unchanged card details revive nothing"
    );
    refresh_uber_card(&env, "mock_mastercard", 2030).await;
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        0,
        "a brand-only change is not a new card"
    );

    refresh_uber_card(&env, "mock_mastercard", 2034).await;
    assert_eq!(
        env.pending_payment_retries(sub_id, invoice_id).await,
        1,
        "a new expiry on the same method revives the retry"
    );

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.process_due_events().await;
    env.run_outbox_and_orchestration().await;

    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Paid);

    let attempts = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions");
    assert!(attempts.iter().any(|t| {
        t.transaction.status == PaymentStatusEnum::Settled
            && t.transaction.payment_method_id == Some(CUST_UBER_PAYMENT_METHOD_ID)
    }));
}

#[rstest]
#[tokio::test]
async fn test_queued_retry_skips_a_hard_declined_method(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let (sub_id, invoice_id) = hard_declined_invoice(&env).await;

    // A retry queued before the hard decline landed.
    let mut conn = env.conn().await;
    env.store()
        .schedule_events(
            &mut conn,
            vec![ScheduledEventNew {
                subscription_id: sub_id,
                tenant_id: TENANT_ID,
                scheduled_time: Utc::now().naive_utc(),
                event_data: ScheduledEventData::RetryPayment { invoice_id },
                source: "dunning".to_string(),
                created_by: Actor::System,
            }],
        )
        .await
        .expect("schedule retry");

    env.set_mock_charge_behavior(MockChargeBehavior::Succeeded)
        .await;
    env.process_due_events().await;
    env.run_outbox_and_orchestration().await;

    let attempts = env
        .store()
        .list_payment_tx_by_invoice_id(TENANT_ID, invoice_id)
        .await
        .expect("list payment transactions");
    assert_eq!(
        attempts.len(),
        1,
        "the expired card must not be charged again"
    );
    env.get_invoices(sub_id)
        .await
        .assert()
        .invoice_at(0)
        .has_payment_status(InvoicePaymentStatus::Errored);
}
