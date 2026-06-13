//! Provable Stripe integration tests against `stripe-mock`.
//!
//! `stripe-mock` (https://github.com/stripe/stripe-mock) serves Stripe's real
//! OpenAPI surface: it validates every inbound request against the published
//! request schema and answers with schema-valid response fixtures. Running our
//! client against it proves two things a pure unit test cannot:
//!
//!   1. Our request *serialization* is accepted by Stripe's schema — including
//!      the awkward bits: `serde_qs` form/bracket encoding, the `#[serde(flatten)]`
//!      mandate keys (`mandate_data[customer_acceptance][...]`), and repeated
//!      `payment_method_types[]` arrays.
//!   2. Our response *deserialization* (`StripePaymentIntent`, `SetupIntent`,
//!      `Customer`, …) matches the shape Stripe actually returns.
//!
//! These are gated behind `STRIPE_MOCK_URL` so CI without stripe-mock simply
//! skips them. To run locally:
//!
//! ```bash
//! stripe-mock -http-port 12111 &
//! STRIPE_MOCK_URL=http://127.0.0.1:12111/ \
//!   cargo test -p stripe-client --test stripe_mock
//! ```

use std::collections::HashMap;
use std::time::Duration;

use secrecy::SecretString;
use stripe_client::client::StripeClient;
use stripe_client::customers::{CreateCustomer, Customer, CustomerApi};
use stripe_client::payment_intents::{
    PaymentIntentApi, PaymentIntentRequest, StripeCaptureMethod,
};
use stripe_client::setup_intents::{
    CreateSetupIntent, CreateSetupIntentUsage, SetupIntentApi, StripeMandateRequest,
    StripeMandateType, StripePaymentMethodType,
};

/// Returns a client pointed at stripe-mock, or `None` when `STRIPE_MOCK_URL`
/// is unset so the suite skips cleanly in environments without stripe-mock.
fn mock_client() -> Option<(StripeClient, SecretString)> {
    let url = std::env::var("STRIPE_MOCK_URL").ok()?;
    let client = StripeClient::from_parts(
        url.as_str(),
        Duration::from_secs(5),
        Duration::from_secs(15),
    );
    Some((client, SecretString::from("sk_test_123".to_string())))
}

macro_rules! client_or_skip {
    () => {
        match mock_client() {
            Some(c) => c,
            None => {
                eprintln!("skipping: STRIPE_MOCK_URL not set");
                return;
            }
        }
    };
}

#[tokio::test]
async fn create_customer_roundtrips() {
    let (client, key) = client_or_skip!();

    let customer: Customer = client
        .create_customer(
            CreateCustomer {
                name: Some("Contract Test".to_string()),
                email: Some("ct@example.invalid".to_string()),
                address: None,
                description: None,
                metadata: None,
                phone: None,
                preferred_locales: None,
                shipping: None,
                source: None,
                validate: None,
            },
            &key,
            "idem-cust-1".to_string(),
        )
        .await
        .expect("create_customer should deserialize against stripe-mock");

    assert!(
        customer.id.starts_with("cus_"),
        "expected a customer id, got {:?}",
        customer.id
    );
}

#[tokio::test]
async fn create_payment_intent_roundtrips() {
    let (client, key) = client_or_skip!();

    // Exercises serde_qs encoding of: metadata map, repeated payment_method_types,
    // bool flags, and the capture_method enum.
    let mut metadata = HashMap::new();
    metadata.insert("meteroid_transaction_id".to_string(), "tx_123".to_string());

    let pi = client
        .create_payment_intent(
            PaymentIntentRequest {
                amount: 1999,
                currency: "eur".to_string(),
                metadata,
                return_url: None,
                confirm: true,
                payment_method: "pm_card_visa".to_string(),
                customer: None,
                setup_mandate_details: None,
                capture_method: StripeCaptureMethod::Automatic,
                off_session: Some(true),
                payment_method_types: vec![StripePaymentMethodType::Card],
            },
            &key,
            "idem-pi-1".to_string(),
        )
        .await
        .expect("create_payment_intent should deserialize against stripe-mock");

    assert!(pi.id.starts_with("pi_"), "expected a pi id, got {}", pi.id);
    assert_eq!(pi.currency, "eur");

    // Prove the read path the reconciliation worker relies on also parses.
    let fetched = client
        .get_payment_intent(&pi.id, &key)
        .await
        .expect("get_payment_intent should deserialize against stripe-mock");
    assert!(fetched.id.starts_with("pi_"));
}

#[tokio::test]
async fn create_setup_intent_with_mandate_roundtrips() {
    let (client, key) = client_or_skip!();

    // The mandate request flattens into bracket-notation keys like
    // `mandate_data[customer_acceptance][online][ip_address]`. stripe-mock
    // rejects malformed shapes, so a 200 here proves the encoding is correct.
    let mut metadata = HashMap::new();
    metadata.insert("meteroid_connection_id".to_string(), "conn_123".to_string());

    let si = client
        .create_setup_intent(
            CreateSetupIntent {
                customer: None,
                setup_mandate_details: Some(StripeMandateRequest::new(StripeMandateType::Online {
                    ip_address: "127.0.0.1".to_string(),
                    user_agent: "meteroid-test".to_string(),
                })),
                payment_method_types: Some(vec![
                    StripePaymentMethodType::Sepa,
                    StripePaymentMethodType::Card,
                ]),
                usage: Some(CreateSetupIntentUsage::OffSession),
                metadata,
            },
            &key,
            "idem-si-1".to_string(),
        )
        .await
        .expect("create_setup_intent should deserialize against stripe-mock");

    assert!(si.id.starts_with("seti_"), "expected a seti id, got {}", si.id);
    assert!(!si.client_secret.is_empty());
}
