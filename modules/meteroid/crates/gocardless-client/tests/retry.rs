//! Regression coverage for `execute`'s handling of `429 Too Many Requests`,
//! driven against a local `wiremock` server (no network).
//!
//! GoCardless rate-limits at ~1000 requests/minute and answers with `429` plus
//! a `Retry-After` header. The client must retry (honoring that header)
//! instead of surfacing a hard error on the first rate-limited attempt.

use std::time::Duration;

use gocardless_client::client::GoCardlessClient;
use gocardless_client::payments::{CreatePayment, CreatePaymentLinks, PaymentApi};
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(base: &str) -> GoCardlessClient {
    GoCardlessClient::from_parts(base, Duration::from_secs(5), Duration::from_secs(5))
}

fn payment_params(mandate: &str) -> CreatePayment {
    CreatePayment {
        amount: 1250,
        currency: "EUR".to_string(),
        description: None,
        metadata: None,
        charge_date: None,
        reference: None,
        links: CreatePaymentLinks {
            mandate: mandate.to_string(),
        },
    }
}

/// A `429` with `Retry-After: 0` must be retried, not surfaced as a hard
/// error on the first attempt.
#[tokio::test]
async fn rate_limited_429_is_retried_and_succeeds() {
    let server = MockServer::start().await;

    let rate_limited_body =
        r#"{"error":{"type":"invalid_state","code":429,"message":"Too many requests"}}"#;
    Mock::given(method("POST"))
        .and(path("/payments"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string(rate_limited_body),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let recovered = r#"{"payments":{"id":"PM1","amount":1250,"currency":"EUR","status":"submitted","links":{"mandate":"MD1"}}}"#;
    Mock::given(method("POST"))
        .and(path("/payments"))
        .respond_with(ResponseTemplate::new(200).set_body_string(recovered))
        .mount(&server)
        .await;

    let payment = client(&server.uri())
        .create_payment(
            payment_params("MD1"),
            &SecretString::from("token"),
            "stable-key",
        )
        .await
        .expect("a 429 with Retry-After must be retried, not surfaced as a hard error");

    assert_eq!(payment.id, "PM1");
}

/// A `429` that exhausts all retry attempts must still surface as an `Api`
/// error (not swallowed or converted into some other error kind).
#[tokio::test]
async fn rate_limited_429_surfaces_after_exhausting_retries() {
    let server = MockServer::start().await;

    let rate_limited_body =
        r#"{"error":{"type":"invalid_state","code":429,"message":"Too many requests"}}"#;
    Mock::given(method("POST"))
        .and(path("/payments"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_string(rate_limited_body),
        )
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .create_payment(
            payment_params("MD1"),
            &SecretString::from("token"),
            "stable-key",
        )
        .await
        .expect_err("a persistently rate-limited call must still fail eventually");

    match err {
        gocardless_client::error::GoCardlessError::Api(e) => assert_eq!(e.http_status, 429),
        other => panic!("expected Api error, got {other:?}"),
    }
}
