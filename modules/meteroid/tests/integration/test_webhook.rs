use crate::data::ids::{CUST_SPOTIFY_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID};
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, CheckoutSessionId, ConnectorId, PaymentTransactionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::checkout_sessions::CheckoutSessionRowNew;
use diesel_models::connectors::ConnectorRowNew;
use diesel_models::enums::{
    CheckoutTypeEnum, ConnectorProviderEnum, ConnectorTypeEnum, PaymentStatusEnum, PaymentTypeEnum,
};
use diesel_models::payments::PaymentTransactionRowNew;
use meteroid::adapters::stripe::Stripe;
use meteroid::workers::pgmq::processors::run_once_webhook_in;
use meteroid_store::domain::enums::PaymentStatusEnum as DomainPaymentStatus;
use meteroid_store::domain::pgmq::{PgmqQueue, WebhookInProcessEvent};
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use std::sync::Arc;
use stripe_client::client::StripeClient;

const ALIAS: &str = "stripe-webhook-test";

/// End-to-end inbound webhook flow: the HTTP handler accepts an unsigned Stripe
/// `payment_intent.succeeded`, dedupes a duplicate delivery, and the WebhookIn
/// worker then dequeues it, reads the archived body from object storage,
/// consolidates the pending transaction to Settled and marks the audit row
/// processed.
#[tokio::test]
async fn test_webhook_in_ingest_dedup_and_worker() {
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    // A Stripe connector with no sensitive data (signature verification skipped),
    // plus a payment awaiting the provider's confirmation.
    let connector_id = ConnectorId::new();
    let checkout_session_id = CheckoutSessionId::new();
    let tx_id = PaymentTransactionId::new();
    {
        let mut conn = setup.store.pool.get().await.unwrap();
        ConnectorRowNew {
            id: connector_id,
            tenant_id: TENANT_ID,
            alias: ALIAS.to_string(),
            connector_type: ConnectorTypeEnum::PaymentProvider,
            provider: ConnectorProviderEnum::Stripe,
            data: None,
            sensitive: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();

        // The transaction is attached to a checkout session; the table requires
        // either an invoice or a checkout session.
        CheckoutSessionRowNew {
            id: checkout_session_id,
            tenant_id: TENANT_ID,
            customer_id: CUST_SPOTIFY_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            billing_start_date: None,
            billing_day_anchor: None,
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
            expires_at: None,
            metadata: None,
            checkout_type: CheckoutTypeEnum::SelfServe,
            subscription_id: None,
            change_date: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();

        PaymentTransactionRowNew {
            id: tx_id,
            tenant_id: TENANT_ID,
            invoice_id: None,
            provider_transaction_id: None,
            amount: 10_000,
            currency: "usd".to_string(),
            payment_method_id: None,
            status: PaymentStatusEnum::Pending,
            payment_type: PaymentTypeEnum::Payment,
            error_type: None,
            processed_at: None,
            checkout_session_id: Some(checkout_session_id),
            pending_plan_version_id: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    // A `payment_intent.succeeded` referencing the pending transaction. The ~10 KB
    // padding keeps the body well over the old 4 KB route limit (regression guard);
    // it lands in an unknown field that the Stripe payload parser ignores.
    let padding = "x".repeat(10 * 1024);
    let body = serde_json::json!({
        "id": "evt_pay",
        "object": "event",
        "type": "payment_intent.succeeded",
        "data": { "object": {
            "object": "payment_intent",
            "id": "pi_pay",
            "amount": 10_000,
            "amount_received": 10_000,
            "currency": "usd",
            "livemode": false,
            "status": "succeeded",
            "description": padding,
            "metadata": {
                "meteroid.tenant_id": TENANT_ID.as_base62(),
                "meteroid.transaction_id": tx_id.as_base62(),
            }
        }}
    });
    assert!(
        serde_json::to_vec(&body).unwrap().len() > 4096,
        "payload must exceed the old 4 KB limit to be a meaningful regression guard"
    );

    let client = reqwest::Client::new();
    let url = format!(
        "{}/webhooks/v1/{}/{}",
        setup.config.rest_api_external_url, TENANT_ID, ALIAS
    );
    let post = || client.post(&url).json(&body).send();

    // First delivery, then a duplicate; both are acked.
    assert_eq!(post().await.unwrap().status(), reqwest::StatusCode::OK);
    assert_eq!(post().await.unwrap().status(), reqwest::StatusCode::OK);

    // Peek the queue (vt = 0 leaves the message visible for the worker): the
    // duplicate was deduped, so exactly one message is enqueued.
    let messages = setup
        .store
        .pgmq_read(
            PgmqQueue::WebhookIn,
            MessageReadQty(10),
            MessageReadVtSec(0),
        )
        .await
        .unwrap();
    assert_eq!(
        messages.len(),
        1,
        "duplicate delivery must be deduped to a single queue message"
    );
    let event: WebhookInProcessEvent = (&messages[0]).try_into().unwrap();
    let webhook_in_event_id = event.webhook_in_event_id;

    // Run the worker once against the same object store the handler archived to.
    run_once_webhook_in(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
        setup.object_store.clone(),
        Arc::new(Stripe {
            client: Arc::new(StripeClient::new()),
        }),
    )
    .await;

    // The pending payment is now settled.
    let mut conn = setup.store.pool.get().await.unwrap();
    let tx = setup
        .store
        .get_payment_tx_by_id_for_update(&mut conn, tx_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(tx.status, DomainPaymentStatus::Settled);

    // The webhook audit row is marked processed.
    let processed = setup
        .services
        .get_webhook_in_event(webhook_in_event_id)
        .await
        .unwrap();
    assert!(
        processed.processed_at.is_some(),
        "worker should mark the event processed"
    );
}
