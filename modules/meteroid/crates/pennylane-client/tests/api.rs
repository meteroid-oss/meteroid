use bytes::Bytes;
use chrono::NaiveDate;
use pennylane_client::client::PennylaneClient;
use pennylane_client::customer_invoices::{Amount, CustomerInvoiceImport};
use pennylane_client::error::PennylaneError;
use rust_decimal_macros::dec;
use secrecy::SecretString;
use std::time::Duration;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> PennylaneClient {
    PennylaneClient::from_parts(
        &server.uri(),
        Duration::from_secs(2),
        Duration::from_secs(2),
        3,
        nonzero_ext::nonzero!(100u32),
    )
}

fn token() -> SecretString {
    SecretString::from("tok_123".to_owned())
}

fn import() -> CustomerInvoiceImport {
    CustomerInvoiceImport {
        file_attachment_id: 1,
        customer_id: 2,
        external_reference: "inv_1".into(),
        invoice_number: "INV-1".into(),
        date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        deadline: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
        currency: "EUR".into(),
        currency_amount_before_tax: Amount(dec!(0.00)),
        currency_amount: Amount(dec!(0.00)),
        currency_tax: Amount(dec!(0.00)),
        amount: None,
        tax: None,
        credited_invoice_id: None,
        transaction_reference: None,
        purchase_order_reference: None,
        invoice_lines: vec![],
    }
}

#[tokio::test]
async fn a_lookup_filters_on_the_external_reference_with_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/external/v2/customers"))
        .and(header("authorization", "Bearer tok_123"))
        .and(query_param(
            "filter",
            r#"[{"field":"external_reference","operator":"eq","value":"cus_1"}]"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"items":[{"id":42,"external_reference":"cus_1","customer_type":"individual"}],"has_more":false,"next_cursor":null}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let found = client(&server)
        .find_customer("cus_1", &token())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, 42);
}

#[tokio::test]
async fn a_rate_limited_read_is_retried_after_the_advertised_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/external/v2/customer_invoices/7"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "0")
                .set_body_string("Rate limit exceeded. Please retry in 0 seconds."),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/external/v2/customer_invoices/7"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":7,"paid":true}"#))
        .mount(&server)
        .await;

    let invoice = client(&server)
        .get_customer_invoice(7, &token())
        .await
        .unwrap();
    assert!(invoice.paid);
}

#[tokio::test]
async fn an_import_is_not_sent_twice_after_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/external/v2/customer_invoices/import"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server)
        .import_customer_invoice(&import(), &token())
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(503));
    assert!(err.is_retryable());
}

#[tokio::test]
async fn a_rate_limited_import_is_sent_again() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/external/v2/customer_invoices/import"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/external/v2/customer_invoices/import"))
        .and(body_partial_json(serde_json::json!({
            "external_reference": "inv_1",
            "currency_amount": "0.00",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"id":9}"#))
        .mount(&server)
        .await;

    let imported = client(&server)
        .import_customer_invoice(&import(), &token())
        .await
        .unwrap();
    assert_eq!(imported.id, 9);
}

#[tokio::test]
async fn a_rejected_import_reports_pennylanes_message() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/external/v2/customer_invoices/import"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_string(r#"{"error":"Entry lines are not balanced","status":422}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server)
        .import_customer_invoice(&import(), &token())
        .await
        .unwrap_err();
    assert!(!err.is_retryable());
    assert_eq!(err.message(), "Entry lines are not balanced");
    assert!(matches!(err, PennylaneError::Api { status: 422, .. }));
}

#[tokio::test]
async fn an_upload_is_retried_on_a_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/external/v2/file_attachments"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/external/v2/file_attachments"))
        .respond_with(ResponseTemplate::new(201).set_body_string(
            r#"{"id":11,"url":null,"filename":"inv.pdf","created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-01T00:00:00Z"}"#,
        ))
        .mount(&server)
        .await;

    let attachment = client(&server)
        .upload_pdf("inv.pdf", Bytes::from_static(b"%PDF-1.7"), &token())
        .await
        .unwrap();
    assert_eq!(attachment.id, 11);

    let received = server.received_requests().await.unwrap();
    let body = String::from_utf8_lossy(&received.last().unwrap().body);
    assert!(body.contains("filename=\"inv.pdf\"") && body.contains("%PDF-1.7"));
}

#[tokio::test]
async fn marking_paid_reads_no_body() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/api/external/v2/customer_invoices/5/mark_as_paid"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client(&server)
        .mark_customer_invoice_as_paid(5, &token())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_refused_token_is_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/external/v2/me"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_token"}"#))
        .expect(1)
        .mount(&server)
        .await;

    let err = client(&server).get_me(&token()).await.unwrap_err();
    assert!(err.is_unauthorized());
}
