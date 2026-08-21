//! Coverage for `CustomerApi::validate_credentials`, driven against a local
//! `wiremock` server (no network).
//!
//! Connector setup must fail fast on a bad/wrong-environment access token
//! instead of persisting it silently — `validate_credentials` is the cheap
//! authenticated call that catches this before the connector is stored.

use std::time::Duration;

use gocardless_client::client::GoCardlessClient;
use gocardless_client::customers::CustomerApi;
use gocardless_client::error::GoCardlessError;
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(base: &str) -> GoCardlessClient {
    GoCardlessClient::from_parts(base, Duration::from_secs(5), Duration::from_secs(5))
}

#[tokio::test]
async fn valid_token_is_accepted() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/customers"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"customers":[],"meta":{}}"#))
        .mount(&server)
        .await;

    client(&server.uri())
        .validate_credentials(&SecretString::from("token"))
        .await
        .expect("a 200 response must validate the token");
}

#[tokio::test]
async fn bad_token_is_rejected() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/customers"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            r#"{"error":{"type":"invalid_api_usage","code":401,"message":"Invalid token"}}"#,
        ))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .validate_credentials(&SecretString::from("bad-token"))
        .await
        .expect_err("an invalid token must not be accepted as valid");

    match err {
        GoCardlessError::Api(e) => assert_eq!(e.http_status, 401),
        other => panic!("expected Api error, got {other:?}"),
    }
}
