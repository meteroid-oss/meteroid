//! Live Stripe **test-mode** integration tests for our `stripe-client` crate.
//!
//! Unlike `stripe_mock.rs` (which validates our request/response shapes against
//! `stripe-mock`'s static OpenAPI fixtures), this suite hits the *real*
//! `api.stripe.com` in **test mode**. That proves the parts a schema mock cannot:
//! Stripe's live business logic actually accepts our `serde_qs` form encoding and
//! returns the real status/decline/`requires_action`/mandate outcomes, and our
//! `StripePaymentIntent` / `SetupIntent` / `RequestError` types deserialize them.
//!
//! It uses Stripe's documented shared test PaymentMethods (`pm_card_visa`,
//! `pm_card_chargeDeclined`, `pm_card_authenticationRequired`). No real money and
//! no real cards are ever involved.
//!
//! Gated on `STRIPE_SECRET_KEY` (an `sk_test_...` key): with the var unset every
//! test returns early and "passes" as a skip, exactly like `stripe_mock.rs` gates
//! on `STRIPE_MOCK_URL`. To run:
//!
//! ```bash
//! export STRIPE_SECRET_KEY=sk_test_...          # test-mode key only
//! cargo test -p stripe-client --test stripe_sandbox -- --nocapture --test-threads=1
//! ```
//!
//! Observed real outcomes at the time of writing (Stripe-Version `2026-04-22.dahlia`):
//!   (a) card success            -> status `succeeded`
//!   (b) card decline            -> HTTP 402 `card_error` / code `card_declined`
//!   (c) 3-D Secure (SCA)        -> status `requires_action` + `next_action` + client_secret
//!   (d) SEPA SetupIntent        -> status `requires_payment_method` (round-trips)
//!       SEPA SetupIntent+mandate-> HTTP 400: `mandate_data` needs `confirm=true`
//!                                  (our `CreateSetupIntent` exposes no `confirm` field)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use secrecy::SecretString;
use stripe_client::client::StripeClient;
use stripe_client::customers::{CreateCustomer, Customer, CustomerApi};
use stripe_client::error::StripeError;
use stripe_client::payment_intents::{
    PaymentIntentApi, PaymentIntentRequest, StripeCaptureMethod, StripePaymentStatus,
};
use stripe_client::setup_intents::{
    CreateSetupIntent, CreateSetupIntentUsage, SetupIntentApi, StripeMandateRequest,
    StripeMandateType, StripePaymentMethodType,
};

/// A client pointed at the real Stripe API plus the `sk_test_...` secret, or
/// `None` when `STRIPE_SECRET_KEY` is unset so the suite skips cleanly.
fn sandbox_client() -> Option<(StripeClient, SecretString)> {
    let key = std::env::var("STRIPE_SECRET_KEY").ok()?;
    if !key.starts_with("sk_test_") {
        // Guard rail: never let this suite fire against a live-mode key.
        eprintln!("skipping: STRIPE_SECRET_KEY is not a test-mode (sk_test_) key");
        return None;
    }
    Some((StripeClient::new(), SecretString::from(key)))
}

macro_rules! client_or_skip {
    () => {
        match sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("skipping: STRIPE_SECRET_KEY not set");
                return;
            }
        }
    };
}

/// Fresh idempotency key per call so re-runs don't replay a cached response.
fn idem(tag: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before unix epoch")
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("sandbox-{tag}-{nanos}-{n}")
}

fn test_metadata() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("meteroid_test".to_string(), "stripe_sandbox".to_string());
    m
}

async fn new_customer(client: &StripeClient, key: &SecretString) -> Customer {
    let customer = client
        .create_customer(
            CreateCustomer {
                name: Some("Meteroid Sandbox".to_string()),
                email: Some("sandbox@example.invalid".to_string()),
                address: None,
                description: Some("stripe-client live sandbox test".to_string()),
                metadata: Some(test_metadata()),
                phone: None,
                preferred_locales: None,
                shipping: None,
                source: None,
                validate: None,
            },
            key,
            idem("customer"),
        )
        .await
        .expect("create_customer should round-trip against live Stripe test mode");

    assert!(
        customer.id.starts_with("cus_"),
        "expected a customer id, got {:?}",
        customer.id
    );
    customer
}

/// (a) Card success: create a customer, then confirm a PaymentIntent with the
/// always-successful `pm_card_visa` -> status `succeeded`.
#[tokio::test]
async fn card_payment_intent_succeeds() {
    let (client, key) = client_or_skip!();

    let customer = new_customer(&client, &key).await;

    let pi = client
        .create_payment_intent(
            PaymentIntentRequest {
                amount: 100,
                currency: "usd".to_string(),
                metadata: test_metadata(),
                return_url: None,
                confirm: true,
                payment_method: "pm_card_visa".to_string(),
                customer: Some(customer.id.clone()),
                setup_mandate_details: None,
                capture_method: StripeCaptureMethod::Automatic,
                off_session: Some(true),
                payment_method_types: vec![StripePaymentMethodType::Card],
            },
            &key,
            idem("pi-success"),
        )
        .await
        .expect("create_payment_intent (pm_card_visa) should succeed");

    eprintln!(
        "(a) card success -> id={} status={:?} amount_received={:?}",
        pi.id, pi.status, pi.amount_received
    );

    assert!(pi.id.starts_with("pi_"), "expected a pi id, got {}", pi.id);
    assert_eq!(pi.status, StripePaymentStatus::Succeeded);
    assert_eq!(pi.amount_received, Some(100));
    assert!(pi.last_payment_error.is_none());

    // The read path the reconciliation worker relies on must also deserialize.
    let fetched = client
        .get_payment_intent(&pi.id, &key)
        .await
        .expect("get_payment_intent should round-trip");
    assert_eq!(fetched.id, pi.id);
    assert_eq!(fetched.status, StripePaymentStatus::Succeeded);
}

/// (b) Card decline: confirming with `pm_card_chargeDeclined` yields HTTP 402.
/// We assert the client *parses* the decline into a `RequestError` (type/code),
/// rather than panicking or losing the error.
#[tokio::test]
async fn card_payment_intent_declines_are_parsed() {
    let (client, key) = client_or_skip!();

    let customer = new_customer(&client, &key).await;

    let result = client
        .create_payment_intent(
            PaymentIntentRequest {
                amount: 100,
                currency: "usd".to_string(),
                metadata: test_metadata(),
                return_url: None,
                confirm: true,
                payment_method: "pm_card_chargeDeclined".to_string(),
                customer: Some(customer.id.clone()),
                setup_mandate_details: None,
                capture_method: StripeCaptureMethod::Automatic,
                off_session: Some(true),
                payment_method_types: vec![StripePaymentMethodType::Card],
            },
            &key,
            idem("pi-decline"),
        )
        .await;

    match result {
        Err(StripeError::Stripe(err)) => {
            eprintln!(
                "(b) card decline -> http={} type={} code={:?} message={:?}",
                err.http_status, err.error_type, err.code, err.message
            );
            assert_eq!(err.http_status, 402, "card declines are 402");
            assert_eq!(err.error_type, "card_error");
            assert_eq!(
                err.code.as_deref(),
                Some("card_declined"),
                "the parsed decline code"
            );
            assert!(
                err.message.is_some(),
                "a human-readable decline message should be parsed"
            );
        }
        other => panic!("expected a parsed Stripe card decline, got {other:?}"),
    }
}

/// (c) 3-D Secure / SCA: confirming `pm_card_authenticationRequired` on-session
/// returns status `requires_action` with a `next_action` and a `client_secret`
/// (the value the portal hands to Stripe.js `handleNextAction`).
#[tokio::test]
async fn card_payment_intent_requires_action_for_3ds() {
    let (client, key) = client_or_skip!();

    let customer = new_customer(&client, &key).await;

    let pi = client
        .create_payment_intent(
            PaymentIntentRequest {
                amount: 100,
                currency: "usd".to_string(),
                metadata: test_metadata(),
                // A return_url is required for on-session redirect-style SCA.
                return_url: Some("https://example.com/return".to_string()),
                confirm: true,
                payment_method: "pm_card_authenticationRequired".to_string(),
                customer: Some(customer.id.clone()),
                setup_mandate_details: None,
                capture_method: StripeCaptureMethod::Automatic,
                // On-session (no off_session): the customer is present to
                // authenticate, so Stripe parks the PI at requires_action rather
                // than hard-declining with `authentication_required`.
                off_session: None,
                payment_method_types: vec![StripePaymentMethodType::Card],
            },
            &key,
            idem("pi-3ds"),
        )
        .await
        .expect("create_payment_intent (3DS) should return a PI, not an error");

    let next_action_type = pi.next_action.as_ref().map(|na| na.action_type.clone());
    eprintln!(
        "(c) 3DS/SCA -> id={} status={:?} has_client_secret={} next_action={:?}",
        pi.id,
        pi.status,
        pi.client_secret.is_some(),
        next_action_type
    );

    assert_eq!(pi.status, StripePaymentStatus::RequiresCustomerAction);
    assert!(
        pi.client_secret.is_some(),
        "requires_action PI must carry a client_secret for SCA"
    );
    let next_action = pi
        .next_action
        .expect("requires_action PI must carry a next_action");
    assert!(
        !next_action.action_type.is_empty(),
        "next_action must have a type (observed: redirect_to_url)"
    );
}

/// (d) SEPA SetupIntent round-trip: `payment_method_types=["sepa_debit"]` with
/// off-session usage. Proves the request serializes and Stripe accepts it, and
/// the `SetupIntent` response (id/status/client_secret/types) deserializes.
#[tokio::test]
async fn sepa_setup_intent_roundtrips() {
    let (client, key) = client_or_skip!();

    let si = client
        .create_setup_intent(
            CreateSetupIntent {
                customer: None,
                setup_mandate_details: None,
                payment_method_types: Some(vec![StripePaymentMethodType::Sepa]),
                usage: Some(CreateSetupIntentUsage::OffSession),
                metadata: test_metadata(),
            },
            &key,
            idem("si-sepa"),
        )
        .await
        .expect("create_setup_intent (sepa_debit) should round-trip against live Stripe");

    eprintln!(
        "(d) SEPA SetupIntent -> id={} status={} usage={} types={:?}",
        si.id, si.status, si.usage, si.payment_method_types
    );

    assert!(
        si.id.starts_with("seti_"),
        "expected a seti id, got {}",
        si.id
    );
    assert!(!si.client_secret.is_empty());
    assert_eq!(si.usage, "off_session");
    assert!(
        si.payment_method_types.iter().any(|t| t == "sepa_debit"),
        "sepa_debit should be echoed back, got {:?}",
        si.payment_method_types
    );
    // A brand-new, unconfirmed SetupIntent awaits a payment method.
    assert_eq!(si.status, "requires_payment_method");
}

/// (d, cont.) SEPA + mandate: the exact `mandate_data[customer_acceptance][...]`
/// bracket encoding from `create_setup_intent_with_mandate_roundtrips`. Against
/// the *real* API (vs stripe-mock) this is rejected with HTTP 400 because
/// `mandate_data` requires `confirm=true`, and `CreateSetupIntent` exposes no
/// `confirm`/`payment_method` field. That the error is `invalid_request_error`
/// naming `mandate_data` (not "unknown parameter") proves Stripe *understood* our
/// flattened mandate keys, and that our client parses the live error response.
#[tokio::test]
async fn sepa_setup_intent_with_mandate_requires_confirm() {
    let (client, key) = client_or_skip!();

    let result = client
        .create_setup_intent(
            CreateSetupIntent {
                customer: None,
                setup_mandate_details: Some(StripeMandateRequest::new(StripeMandateType::Online {
                    ip_address: "127.0.0.1".to_string(),
                    user_agent: "meteroid-sandbox-test".to_string(),
                })),
                payment_method_types: Some(vec![StripePaymentMethodType::Sepa]),
                usage: Some(CreateSetupIntentUsage::OffSession),
                metadata: test_metadata(),
            },
            &key,
            idem("si-sepa-mandate"),
        )
        .await;

    match result {
        // Expected on the real API: our request was well-formed, business rule rejects it.
        Err(StripeError::Stripe(err)) => {
            eprintln!(
                "(d) SEPA+mandate -> http={} type={} code={:?} message={:?}",
                err.http_status, err.error_type, err.code, err.message
            );
            assert_eq!(err.http_status, 400);
            assert_eq!(err.error_type, "invalid_request_error");
            assert!(
                err.message
                    .as_deref()
                    .unwrap_or_default()
                    .contains("mandate_data"),
                "Stripe should reference the mandate_data param it parsed, got {:?}",
                err.message
            );
        }
        // If a future client gains a `confirm` field, accept a clean round-trip too.
        Ok(si) => {
            eprintln!("(d) SEPA+mandate -> unexpectedly accepted: id={}", si.id);
            assert!(si.id.starts_with("seti_"));
        }
        other => panic!("expected a parsed Stripe error or a SetupIntent, got {other:?}"),
    }
}
