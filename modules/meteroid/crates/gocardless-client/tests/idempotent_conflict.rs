//! End-to-end coverage of the GoCardless `409 idempotent_creation_conflict`
//! recovery path, driven against a local `wiremock` server (no network).
//!
//! GoCardless does NOT replay the original 2xx on an idempotent retry: it
//! answers 409 `invalid_state` with `reason=idempotent_creation_conflict` and a
//! `conflicting_resource_id`. The client must GET that resource and return it as
//! success — otherwise a client-side timeout on a successful create wedges the
//! stable idempotency key and risks a double-charge via dunning.

use std::time::Duration;

use gocardless_client::client::GoCardlessClient;
use gocardless_client::error::GoCardlessError;
use gocardless_client::payments::{CreatePayment, CreatePaymentLinks, PaymentApi};
use secrecy::SecretString;
use wiremock::matchers::{header, method, path};
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

const CONFLICT_BODY: &str = r#"{
    "error": {
        "type": "invalid_state",
        "code": 409,
        "message": "A resource has already been created with this idempotency key",
        "request_id": "req_abc",
        "errors": [{
            "reason": "idempotent_creation_conflict",
            "message": "A resource has already been created with this idempotency key",
            "links": { "conflicting_resource_id": "PM0RECOVERED1" }
        }]
    }
}"#;

/// POST → 409 idempotent_creation_conflict, then GET the conflicting id → 200.
/// The create call must resolve to the recovered payment.
#[tokio::test]
async fn create_payment_recovers_on_idempotent_conflict() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/payments"))
        .and(header("Idempotency-Key", "stable-key"))
        .respond_with(ResponseTemplate::new(409).set_body_string(CONFLICT_BODY))
        .mount(&server)
        .await;

    let recovered = r#"{"payments":{"id":"PM0RECOVERED1","amount":1250,"currency":"EUR","status":"submitted","links":{"mandate":"MD1"}}}"#;
    Mock::given(method("GET"))
        .and(path("/payments/PM0RECOVERED1"))
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
        .expect("409 idempotent conflict must recover to the existing payment");

    assert_eq!(payment.id, "PM0RECOVERED1");
    assert_eq!(payment.amount, 1250);
    assert_eq!(payment.links.mandate.as_deref(), Some("MD1"));
}

/// A 409 that is NOT an idempotent conflict (different reason) must surface as a
/// hard `Api` error, and must not trigger any recovery GET.
#[tokio::test]
async fn other_409_does_not_recover() {
    let server = MockServer::start().await;

    let body = r#"{
        "error": {
            "type": "invalid_state",
            "code": 409,
            "message": "Mandate is not active",
            "errors": [{ "reason": "mandate_is_inactive" }]
        }
    }"#;
    Mock::given(method("POST"))
        .and(path("/payments"))
        .and(header("Idempotency-Key", "stable-key"))
        .respond_with(ResponseTemplate::new(409).set_body_string(body))
        .mount(&server)
        .await;
    // No GET mock is mounted: a recovery GET would 404 on the mock server and
    // fail the test with a decode error instead of the expected Api error.

    let err = client(&server.uri())
        .create_payment(
            payment_params("MD1"),
            &SecretString::from("token"),
            "stable-key",
        )
        .await
        .expect_err("non-idempotent 409 must remain an error");

    match err {
        GoCardlessError::Api(e) => {
            assert_eq!(e.http_status, 409);
            assert!(!e.is_idempotent_creation_conflict());
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

/// A generic 4xx (422 validation) must surface as an `Api` error, never recovery.
#[tokio::test]
async fn validation_422_does_not_recover() {
    let server = MockServer::start().await;

    let body = r#"{"error":{"type":"validation_failed","code":422,"message":"Validation failed","errors":[{"field":"amount","message":"is required"}]}}"#;
    Mock::given(method("POST"))
        .and(path("/payments"))
        .and(header("Idempotency-Key", "stable-key"))
        .respond_with(ResponseTemplate::new(422).set_body_string(body))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .create_payment(
            payment_params("MD1"),
            &SecretString::from("token"),
            "stable-key",
        )
        .await
        .expect_err("422 must remain an error");

    match err {
        GoCardlessError::Api(e) => assert_eq!(e.http_status, 422),
        other => panic!("expected Api error, got {other:?}"),
    }
}
