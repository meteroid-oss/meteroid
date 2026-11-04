//! Refund create/list against a local `wiremock` server (no network).

use std::time::Duration;

use gocardless_client::client::GoCardlessClient;
use gocardless_client::refunds::{CreateRefund, CreateRefundLinks, RefundApi, RefundStatus};
use secrecy::SecretString;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(base: &str) -> GoCardlessClient {
    GoCardlessClient::from_parts(base, Duration::from_secs(5), Duration::from_secs(5))
}

const REFUND: &str = r#"{"id":"RF1","amount":500,"currency":"EUR","reference":null,
    "status":"pending_submission","metadata":{"meteroid.refund_id":"r1"},"links":{"payment":"PM1"}}"#;

#[tokio::test]
async fn create_refund_posts_the_wrapped_body_under_the_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/refunds"))
        .and(header("Idempotency-Key", "refund:r1"))
        .and(body_json(serde_json::json!({"refunds": {
            "amount": 500,
            "total_amount_confirmation": 800,
            "metadata": {"meteroid.refund_id": "r1"},
            "links": {"payment": "PM1"}
        }})))
        .respond_with(
            ResponseTemplate::new(201).set_body_string(format!(r#"{{"refunds":{REFUND}}}"#)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let refund = client(&server.uri())
        .create_refund(
            CreateRefund {
                amount: 500,
                total_amount_confirmation: 800,
                reference: None,
                metadata: Some([("meteroid.refund_id".to_string(), "r1".to_string())].into()),
                links: CreateRefundLinks {
                    payment: "PM1".to_string(),
                },
            },
            &SecretString::from("token"),
            "refund:r1",
        )
        .await
        .unwrap();

    assert_eq!(refund.id, "RF1");
    assert_eq!(refund.status, RefundStatus::PendingSubmission);
}

#[tokio::test]
async fn list_payment_refunds_filters_by_payment() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refunds"))
        .and(query_param("payment", "PM1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"refunds":[{REFUND}],"meta":{{"cursors":{{"before":null,"after":null}},"limit":50}}}}"#
            )),
        )
        .expect(1)
        .mount(&server)
        .await;

    let refunds = client(&server.uri())
        .list_payment_refunds("PM1", &SecretString::from("token"))
        .await
        .unwrap();

    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].metadata["meteroid.refund_id"], "r1");
}
