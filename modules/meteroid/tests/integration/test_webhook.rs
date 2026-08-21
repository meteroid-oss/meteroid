use crate::data::ids::{CUST_SPOTIFY_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID};
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CheckoutSessionId, PaymentTransactionId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel_models::checkout_sessions::CheckoutSessionRowNew;
use diesel_models::enums::{CheckoutTypeEnum, PaymentStatusEnum, PaymentTypeEnum};
use diesel_models::payments::PaymentTransactionRowNew;
use meteroid::workers::pgmq::processors::run_once_webhook_in;
use meteroid_store::domain::connectors::{
    GocardlessPublicData, GocardlessSensitiveData, StripeSensitiveData,
};
use meteroid_store::domain::enums::PaymentStatusEnum as DomainPaymentStatus;
use meteroid_store::domain::pgmq::{PgmqQueue, WebhookInProcessEvent};
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use std::sync::Arc;

const ALIAS: &str = "stripe-webhook-test";
const WEBHOOK_SECRET: &str = "whsec_integration_test_secret";

/// Stripe's signing scheme: `t=<unix_ts>,v1=<hex hmac_sha256(secret, "<ts>.<body>")>`.
fn stripe_signature(body: &str, secret: &str) -> String {
    let ts = chrono::Utc::now().timestamp();
    let mac = hmac_sha256::HMAC::mac(format!("{ts}.{body}").as_bytes(), secret.as_bytes());
    format!("t={ts},v1={}", hex::encode(mac))
}

/// End-to-end inbound webhook flow: the HTTP handler verifies a signed Stripe
/// `payment_intent.succeeded`, dedupes a duplicate delivery, and the WebhookIn
/// worker then settles the pending transaction and marks the audit row processed.
#[tokio::test]
async fn test_webhook_in_ingest_dedup_and_worker() {
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    // A Stripe connector whose (encrypted at rest) sensitive data carries the
    // webhook signing secret — verification is mandatory at ingest.
    setup
        .store
        .connect_stripe(
            Actor::System,
            TENANT_ID,
            ALIAS.to_string(),
            "pk_test_123".to_string(),
            StripeSensitiveData {
                api_secret_key: "sk_test_123".to_string(),
                webhook_secret: WEBHOOK_SECRET.to_string(),
                webhook_endpoint_id: None,
            },
            "acct_test_123".to_string(),
        )
        .await
        .unwrap();

    // A payment awaiting the provider's confirmation.
    let checkout_session_id = CheckoutSessionId::new();
    let tx_id = PaymentTransactionId::new();
    {
        let mut conn = setup.store.pool.get().await.unwrap();

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
            next_action: None,
            initiated_by_customer_id: None,
        }
        .insert(&mut conn)
        .await
        .unwrap();
    }

    // A `payment_intent.succeeded` referencing the pending transaction. The ~10 KB
    // padding keeps the body well over the old 4 KB route limit (regression guard);
    // it lands in an unknown field that the Stripe payload parser ignores.
    let padding = "x".repeat(10 * 1024);
    let payload = serde_json::json!({
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
    let body = serde_json::to_string(&payload).unwrap();
    assert!(
        body.len() > 4096,
        "payload must exceed the old 4 KB limit to be a meaningful regression guard"
    );

    // Sign the exact bytes we send with the secret stored in the connector.
    let signature = stripe_signature(&body, WEBHOOK_SECRET);

    let client = reqwest::Client::new();
    let url = format!(
        "{}/webhooks/v1/{}/{}",
        setup.config.rest_api_external_url, TENANT_ID, ALIAS
    );
    let post = || {
        client
            .post(&url)
            .header("Stripe-Signature", signature.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .send()
    };

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

// ── GoCardless inbound webhook coverage ────────────────────────────────

const GC_ALIAS: &str = "gocardless-webhook-test";
const GC_WEBHOOK_SECRET: &str = "gc_whsec_integration_test";

/// GoCardless signs the raw body with a plain hex HMAC-SHA-256 in the
/// `Webhook-Signature` header (no timestamp).
fn gocardless_signature(body: &str, secret: &str) -> String {
    hex::encode(hmac_sha256::HMAC::mac(body.as_bytes(), secret.as_bytes()))
}

async fn start_meteroid_with_gocardless() -> meteroid_it::container::MeteroidSetup {
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    setup
        .store
        .connect_gocardless(
            TENANT_ID,
            GC_ALIAS.to_string(),
            GocardlessPublicData {
                creditor_id: Some("CR123".to_string()),
                environment: "sandbox".to_string(),
            },
            GocardlessSensitiveData {
                access_token: "sandbox_token".to_string(),
                webhook_secret: GC_WEBHOOK_SECRET.to_string(),
            },
        )
        .await
        .unwrap();

    setup
}

/// Insert a Pending transaction (attached to a fresh checkout session) whose
/// `provider_transaction_id` is the GoCardless payment id — the state after an
/// async DD charge, before the settlement webhook arrives.
async fn insert_pending_gc_tx(
    setup: &meteroid_it::container::MeteroidSetup,
    tx_id: PaymentTransactionId,
    provider_transaction_id: &str,
) {
    let mut conn = setup.store.pool.get().await.unwrap();
    let checkout_session_id = CheckoutSessionId::new();

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
        provider_transaction_id: Some(provider_transaction_id.to_string()),
        amount: 10_000,
        currency: "eur".to_string(),
        payment_method_id: None,
        status: PaymentStatusEnum::Pending,
        payment_type: PaymentTypeEnum::Payment,
        error_type: None,
        processed_at: None,
        checkout_session_id: Some(checkout_session_id),
        pending_plan_version_id: None,
        next_action: None,
        initiated_by_customer_id: None,
    }
    .insert(&mut conn)
    .await
    .unwrap();
}

/// A settlement webhook whose events carry EMPTY metadata (the real GoCardless
/// shape) must still settle the local transaction, resolved by the provider
/// payment id in `links.payment`. A duplicate delivery of the same batch is
/// deduped per event, so it never reprocesses.
#[tokio::test]
async fn test_gocardless_settles_via_provider_id_and_dedups() {
    let setup = start_meteroid_with_gocardless().await;

    let tx_id = PaymentTransactionId::new();
    insert_pending_gc_tx(&setup, tx_id, "PM_GC_SETTLE_1").await;

    let body =
        include_str!("fixtures/webhooks/gocardless/gocardless_confirmed_empty_metadata.json");
    let signature = gocardless_signature(body, GC_WEBHOOK_SECRET);

    let client = reqwest::Client::new();
    let url = format!(
        "{}/webhooks/v1/{}/{}",
        setup.config.rest_api_external_url, TENANT_ID, GC_ALIAS
    );
    let post = || {
        client
            .post(&url)
            .header("Webhook-Signature", signature.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
    };

    // First delivery + a duplicate; both acked, but the second is deduped on
    // the inner event's `EV...` id.
    assert_eq!(post().await.unwrap().status(), reqwest::StatusCode::OK);
    assert_eq!(post().await.unwrap().status(), reqwest::StatusCode::OK);

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
        "duplicate batch delivery must dedup to a single queue message per event"
    );

    run_once_webhook_in(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
        setup.object_store.clone(),
    )
    .await;

    let mut conn = setup.store.pool.get().await.unwrap();
    let tx = setup
        .store
        .get_payment_tx_by_id_for_update(&mut conn, tx_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(
        tx.status,
        DomainPaymentStatus::Settled,
        "empty-metadata settlement must resolve the tx by provider payment id"
    );
}

/// The real GoCardless payment shape carries our ids in `resource_metadata`
/// (the event's own `metadata` is empty). Settlement must resolve the local
/// transaction directly from `resource_metadata.meteroid.transaction_id` — not
/// the provider-id fallback — and stamp the settlement time from the event's
/// `created_at`, not wall-clock.
#[tokio::test]
async fn test_gocardless_settles_via_resource_metadata() {
    let setup = start_meteroid_with_gocardless().await;

    let tx_id = PaymentTransactionId::new();
    // provider_transaction_id deliberately does NOT match links.payment, so the
    // settlement can only land via the resource_metadata transaction id.
    insert_pending_gc_tx(&setup, tx_id, "PM_UNRELATED_RM").await;

    let payload = serde_json::json!({
        "events": [{
            "id": "EV_GC_RM_SETTLE_1",
            "created_at": "2026-07-20T10:00:00.000Z",
            "resource_type": "payments",
            "action": "confirmed",
            "links": { "payment": "PM_GC_RM_NOMATCH" },
            "metadata": {},
            "resource_metadata": { "meteroid.transaction_id": tx_id.as_base62() }
        }]
    });
    let body = serde_json::to_string(&payload).unwrap();
    let signature = gocardless_signature(&body, GC_WEBHOOK_SECRET);

    let client = reqwest::Client::new();
    let url = format!(
        "{}/webhooks/v1/{}/{}",
        setup.config.rest_api_external_url, TENANT_ID, GC_ALIAS
    );
    let response = client
        .post(&url)
        .header("Webhook-Signature", signature.as_str())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    run_once_webhook_in(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
        setup.object_store.clone(),
    )
    .await;

    let mut conn = setup.store.pool.get().await.unwrap();
    let tx = setup
        .store
        .get_payment_tx_by_id_for_update(&mut conn, tx_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(
        tx.status,
        DomainPaymentStatus::Settled,
        "resource_metadata transaction id must settle the tx directly"
    );

    // Settlement time is the event's own `created_at`, not wall-clock.
    let expected = chrono::NaiveDate::from_ymd_opt(2026, 7, 20)
        .unwrap()
        .and_hms_opt(10, 0, 0)
        .unwrap();
    assert_eq!(
        tx.processed_at,
        Some(expected),
        "processed_at must come from the GoCardless event created_at"
    );
}

/// A poison event first, a legitimate settlement second, in one batch. Splitting
/// at ingest puts each in its own audit row + pgmq message, so the poison can't
/// block the valid settlement and both rows are acked (poison as a no-op).
#[tokio::test]
async fn test_gocardless_poison_event_does_not_block_valid_settlement() {
    let setup = start_meteroid_with_gocardless().await;

    let valid_tx_id = PaymentTransactionId::new();
    insert_pending_gc_tx(&setup, valid_tx_id, "PM_GC_VALID_1").await;

    let body = include_str!("fixtures/webhooks/gocardless/gocardless_batch_poison_then_valid.json");
    let signature = gocardless_signature(body, GC_WEBHOOK_SECRET);

    let client = reqwest::Client::new();
    let url = format!(
        "{}/webhooks/v1/{}/{}",
        setup.config.rest_api_external_url, TENANT_ID, GC_ALIAS
    );
    let response = client
        .post(&url)
        .header("Webhook-Signature", signature.as_str())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // The batch of two events splits into two independent queue messages.
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
        2,
        "a two-event batch must split into two audit rows / queue messages"
    );
    let event_ids: Vec<_> = messages
        .iter()
        .map(|m| {
            let e: WebhookInProcessEvent = m.try_into().unwrap();
            e.webhook_in_event_id
        })
        .collect();

    run_once_webhook_in(
        Arc::new(setup.store.clone()),
        Arc::new(setup.services.clone()),
        setup.object_store.clone(),
    )
    .await;

    // The valid settlement lands despite the sibling poison event.
    let mut conn = setup.store.pool.get().await.unwrap();
    let tx = setup
        .store
        .get_payment_tx_by_id_for_update(&mut conn, valid_tx_id, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(tx.status, DomainPaymentStatus::Settled);

    // Both rows are acked — the poison one as a no-op, not a permanent failure
    // that would wedge the queue — and the poison row records why it was
    // discarded for forensics.
    for id in event_ids {
        let row = setup.services.get_webhook_in_event(id).await.unwrap();
        assert!(
            row.processed_at.is_some(),
            "every split event row (including the poison one) must be acked"
        );
        if row.event_id.as_deref() == Some("EV_GC_POISON_1") {
            assert!(
                row.error
                    .as_deref()
                    .is_some_and(|e| e.contains("discarded non-retryable event")),
                "an acked poison event must record the discard reason on the audit row"
            );
        } else {
            assert!(
                row.error.is_none(),
                "a cleanly handled event must not carry an error"
            );
        }
    }
}
