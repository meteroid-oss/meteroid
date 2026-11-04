use secrecy::SecretString;
use stancer_client::client::StancerClient;
use stancer_client::error::StancerError;
use stancer_client::refunds::{CreateRefund, RefundStatus};
use std::time::Duration;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(base: &str) -> StancerClient {
    StancerClient::from_parts(base, Duration::from_secs(5), Duration::from_secs(5))
}

const REFUND: &str = r#"{"id":"refd_1","payment":"paym_1","amount":500,"currency":"eur",
    "status":"to_refund","created":1726000000,"payout":null}"#;

#[tokio::test]
async fn create_refund_posts_payment_and_amount() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/refunds/"))
        .and(body_json(
            serde_json::json!({"payment": "paym_1", "amount": 500}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(REFUND))
        .expect(1)
        .mount(&server)
        .await;

    let refund = client(&server.uri())
        .create_refund(
            CreateRefund {
                payment: "paym_1".to_string(),
                amount: Some(500),
            },
            &SecretString::from("stest_key"),
        )
        .await
        .unwrap();

    assert_eq!(refund.id, "refd_1");
    assert_eq!(refund.amount, 500);
    assert_eq!(refund.status, RefundStatus::ToRefund);
}

#[tokio::test]
async fn create_refund_is_not_retried_on_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/refunds/"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .create_refund(
            CreateRefund {
                payment: "paym_1".to_string(),
                amount: Some(500),
            },
            &SecretString::from("stest_key"),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, StancerError::Stancer(e) if e.http_status == 503));
}

#[tokio::test]
async fn get_refund_reads_the_refund_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/refunds/refd_1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(REFUND.replace("to_refund", "refunded")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let refund = client(&server.uri())
        .get_refund("refd_1", &SecretString::from("stest_key"))
        .await
        .unwrap();

    assert_eq!(refund.payment, "paym_1");
    assert_eq!(refund.status, RefundStatus::Refunded);
}
