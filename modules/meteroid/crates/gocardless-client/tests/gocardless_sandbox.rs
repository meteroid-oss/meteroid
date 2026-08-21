//! Gated integration test exercising the `gocardless-client` crate against the
//! REAL GoCardless SANDBOX (`https://api-sandbox.gocardless.com`).
//!
//! Proves our request serialization and response parsing work against the live
//! API: customer create, mandate setup (Billing Request + Flow, and a direct
//! IBAN bank-account + mandate), a real payment, a parsed API error, and
//! webhook envelope parsing.
//!
//! The networked test skips (green) when `GOCARDLESS_ACCESS_TOKEN` is unset. Run:
//!   source sandbox-creds.env
//!   cargo test -p gocardless-client --test gocardless_sandbox -- --nocapture --test-threads=1
//!
//! `GOCARDLESS_API_BASE` optionally overrides the API base URL (default
//! `https://api-sandbox.gocardless.com`). It exists so the suite can run in
//! environments whose egress proxy is incompatible with reqwest's HTTPS
//! tunneling: point it at a transparent relay that forwards verbatim to the
//! real sandbox. The client's request serialization and response parsing are
//! unchanged — only the transport hop differs.

use std::collections::HashMap;
use std::time::Duration;

use gocardless_client::billing_requests::{
    BillingRequestApi, BillingRequestFlowLinks, BillingRequestLinks, CreateBillingRequest,
    CreateBillingRequestFlow, MandateRequest,
};
use gocardless_client::client::GoCardlessClient;
use gocardless_client::customers::{CreateCustomer, CustomerApi};
use gocardless_client::error::GoCardlessError;
use gocardless_client::mandates::{MandateApi, MandateStatus};
use gocardless_client::payments::{CreatePayment, CreatePaymentLinks, PaymentApi, PaymentStatus};
use gocardless_client::webhook::GoCardlessWebhook;
use secrecy::SecretString;

const SANDBOX_BASE: &str = "https://api-sandbox.gocardless.com";
/// GoCardless-published SEPA test IBAN (Commerzbank, DE).
const TEST_IBAN: &str = "DE89370400440532013000";

fn idem() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// API base: `GOCARDLESS_API_BASE` if set, else the real sandbox.
fn api_base() -> String {
    std::env::var("GOCARDLESS_API_BASE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| SANDBOX_BASE.to_string())
}

fn meta(scenario: &str) -> HashMap<String, String> {
    HashMap::from([("meteroid.scenario".to_string(), scenario.to_string())])
}

fn new_customer_params() -> CreateCustomer {
    CreateCustomer {
        email: Some(format!("sandbox+{}@example.com", uuid::Uuid::new_v4())),
        given_name: Some("Jane".to_string()),
        family_name: Some("Doe".to_string()),
        company_name: Some("Meteroid Sandbox Test".to_string()),
        language: Some("en".to_string()),
        phone_number: None,
        address_line1: Some("1 Test Street".to_string()),
        address_line2: None,
        address_line3: None,
        city: Some("Berlin".to_string()),
        region: None,
        postal_code: Some("10115".to_string()),
        country_code: Some("DE".to_string()),
        metadata: Some(meta("customer")),
    }
}

/// Raw POST used only for endpoints our client does not (yet) wrap:
/// `customer_bank_accounts` and mandate creation. Mirrors the client's headers.
async fn raw_post(
    base: &str,
    path: &str,
    body: serde_json::Value,
    token: &str,
) -> (u16, serde_json::Value) {
    let resp = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .header("GoCardless-Version", "2015-07-06")
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", idem())
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("sandbox request should reach the network");
    let status = resp.status().as_u16();
    let json = resp
        .json::<serde_json::Value>()
        .await
        .expect("sandbox response should be JSON");
    (status, json)
}

#[tokio::test]
async fn sandbox_end_to_end() {
    let Some(raw_token) = std::env::var("GOCARDLESS_ACCESS_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    else {
        eprintln!("SKIP sandbox_end_to_end: GOCARDLESS_ACCESS_TOKEN unset");
        return;
    };
    let token = SecretString::from(raw_token.clone());
    let base = api_base();
    let client = if base == SANDBOX_BASE {
        GoCardlessClient::from_sandbox()
    } else {
        eprintln!("[env] routing client via relay base = {base}");
        GoCardlessClient::from_parts(&base, Duration::from_secs(10), Duration::from_secs(30))
    };

    // ── (a) Create customer ────────────────────────────────────────────
    let customer = client
        .create_customer(new_customer_params(), &token, &idem())
        .await
        .expect("create_customer must succeed against sandbox");
    assert!(
        customer.id.starts_with("CU"),
        "expected CU… customer id, got {}",
        customer.id
    );
    eprintln!(
        "[a] customer created: id={} country={:?}",
        customer.id, customer.country_code
    );

    // ── (b) Mandate setup path 1: Billing Request + Flow (our client) ───
    let br = client
        .create_billing_request(
            CreateBillingRequest {
                mandate_request: Some(MandateRequest {
                    currency: "EUR".to_string(),
                    scheme: Some("sepa_core".to_string()),
                    description: Some("Sandbox SEPA mandate".to_string()),
                    metadata: Some(meta("mandate_setup")),
                }),
                payment_request: None,
                metadata: Some(meta("mandate_setup")),
                links: Some(BillingRequestLinks {
                    customer: Some(customer.id.clone()),
                    creditor: None,
                }),
            },
            &token,
            &idem(),
        )
        .await
        .expect("create_billing_request must succeed");
    assert!(
        br.id.starts_with("BRQ"),
        "expected BRQ… billing request id, got {}",
        br.id
    );
    eprintln!("[b] billing_request: id={} status={:?}", br.id, br.status);

    let flow = client
        .create_billing_request_flow(
            CreateBillingRequestFlow {
                redirect_uri: None,
                exit_uri: None,
                lock_currency: Some(true),
                lock_bank_account: None,
                auto_fulfil: None,
                links: BillingRequestFlowLinks {
                    billing_request: br.id.clone(),
                },
            },
            &token,
            &idem(),
        )
        .await
        .expect("create_billing_request_flow must succeed");
    assert!(
        flow.id.starts_with("BRF"),
        "expected BRF… flow id, got {}",
        flow.id
    );
    assert!(
        flow.authorisation_url.starts_with("https://"),
        "expected an https authorisation_url, got {}",
        flow.authorisation_url
    );
    assert_eq!(flow.links.billing_request.as_deref(), Some(br.id.as_str()));
    eprintln!(
        "[b] billing_request_flow: id={} authorisation_url={} expires_at={:?}",
        flow.id, flow.authorisation_url, flow.expires_at
    );

    // ── (b) Mandate setup path 2: direct IBAN bank account + mandate ────
    // Our client wraps neither endpoint, so use raw reqwest, then read the
    // mandate back THROUGH our client to prove `Mandate` deserialization.
    let (ba_status, ba) = raw_post(
        &base,
        "/customer_bank_accounts",
        serde_json::json!({
            "customer_bank_accounts": {
                "account_holder_name": "Jane Doe",
                "iban": TEST_IBAN,
                "country_code": "DE",
                "links": { "customer": customer.id }
            }
        }),
        &raw_token,
    )
    .await;
    assert_eq!(ba_status, 201, "bank account create failed: {ba}");
    let ba_id = ba["customer_bank_accounts"]["id"]
        .as_str()
        .expect("bank account id")
        .to_string();
    assert!(ba_id.starts_with("BA"), "expected BA… id, got {ba_id}");
    eprintln!(
        "[b] customer_bank_account: id={ba_id} bank={:?}",
        ba["customer_bank_accounts"]["bank_name"]
    );

    let (md_status, md) = raw_post(
        &base,
        "/mandates",
        serde_json::json!({
            "mandates": {
                "scheme": "sepa_core",
                "links": { "customer_bank_account": ba_id }
            }
        }),
        &raw_token,
    )
    .await;
    assert_eq!(md_status, 201, "mandate create failed: {md}");
    let mandate_id = md["mandates"]["id"]
        .as_str()
        .expect("mandate id")
        .to_string();
    assert!(
        mandate_id.starts_with("MD"),
        "expected MD… id, got {mandate_id}"
    );
    eprintln!(
        "[b] mandate created (raw): id={mandate_id} status={}",
        md["mandates"]["status"]
    );

    // Read it back through OUR client → exercises MandateApi + Mandate parsing.
    let mandate = client
        .get_mandate(&mandate_id, &token)
        .await
        .expect("get_mandate must parse the sandbox mandate");
    assert_eq!(mandate.id, mandate_id);
    assert_eq!(mandate.scheme.as_deref(), Some("sepa_core"));
    assert!(
        matches!(
            mandate.status,
            MandateStatus::PendingSubmission
                | MandateStatus::Submitted
                | MandateStatus::PendingCustomerApproval
                | MandateStatus::Active
        ),
        "unexpected fresh mandate status: {:?}",
        mandate.status
    );
    eprintln!(
        "[b] client.get_mandate: status={:?} scheme={:?} next_charge={:?}",
        mandate.status, mandate.scheme, mandate.next_possible_charge_date
    );

    // ── (c) Payment against the mandate (our client) ───────────────────
    let payment = client
        .create_payment(
            CreatePayment {
                amount: 1250,
                currency: "EUR".to_string(),
                description: Some("Sandbox test charge".to_string()),
                metadata: Some(meta("payment")),
                charge_date: None,
                reference: None,
                links: CreatePaymentLinks {
                    mandate: mandate_id.clone(),
                },
            },
            &token,
            &idem(),
        )
        .await
        .expect("create_payment against a fresh sandbox mandate must succeed");
    assert!(
        payment.id.starts_with("PM"),
        "expected PM… payment id, got {}",
        payment.id
    );
    assert_eq!(payment.amount, 1250);
    assert_eq!(payment.currency, "EUR");
    assert!(
        matches!(
            payment.status,
            PaymentStatus::PendingSubmission
                | PaymentStatus::Submitted
                | PaymentStatus::PendingCustomerApproval
        ),
        "unexpected initial payment status: {:?}",
        payment.status
    );
    assert_eq!(payment.links.mandate.as_deref(), Some(mandate_id.as_str()));
    eprintln!(
        "[c] payment created: id={} status={:?} amount={} {}",
        payment.id, payment.status, payment.amount, payment.currency
    );

    // Round-trip GET through our client → exercises PaymentApi::get_payment.
    let fetched = client
        .get_payment(&payment.id, &token)
        .await
        .expect("get_payment must parse the sandbox payment");
    assert_eq!(fetched.id, payment.id);
    eprintln!("[c] client.get_payment: status={:?}", fetched.status);

    // ── (c) Negative: payment on a bogus mandate → PARSED GoCardless error
    let bogus = client
        .create_payment(
            CreatePayment {
                amount: 1250,
                currency: "EUR".to_string(),
                description: None,
                metadata: None,
                charge_date: None,
                reference: None,
                links: CreatePaymentLinks {
                    mandate: "MD999999999999".to_string(),
                },
            },
            &token,
            &idem(),
        )
        .await;
    match bogus {
        Err(GoCardlessError::Api(err)) => {
            assert!(
                err.http_status >= 400,
                "expected a 4xx/5xx status, got {}",
                err.http_status
            );
            assert!(
                err.error_type.is_some() && err.message.is_some(),
                "error body should deserialize type+message, got {err:?}"
            );
            eprintln!(
                "[c-neg] parsed Api error: status={} type={:?} code={:?} message={:?} request_id={:?} detail_reasons={:?}",
                err.http_status,
                err.error_type,
                err.code,
                err.message,
                err.request_id,
                err.errors
                    .iter()
                    .map(|e| e.reason.clone())
                    .collect::<Vec<_>>(),
            );
        }
        other => panic!("expected a parsed GoCardless Api error, got {other:?}"),
    }

    eprintln!("[✓] sandbox_end_to_end complete");
}

/// Part (d): webhook envelope parsing — no network required, always runs.
#[test]
fn webhook_envelope_parses() {
    let body = br#"{
        "events": [
            {
                "id": "EV0000000000A1",
                "created_at": "2026-07-14T02:00:00.000Z",
                "resource_type": "payments",
                "action": "confirmed",
                "links": { "payment": "PM0000000001", "mandate": "MD0000000001", "creditor": "CR0000000001", "organisation": "OR0000000001" },
                "details": { "origin": "bank", "cause": "payment_confirmed", "description": "Payment confirmed" },
                "metadata": { "meteroid.scenario": "payment" }
            },
            {
                "id": "EV0000000000B2",
                "created_at": "2026-07-14T02:05:00.000Z",
                "resource_type": "mandates",
                "action": "active",
                "links": { "mandate": "MD0000000001", "customer": "CU0000000001" },
                "details": { "origin": "gocardless", "cause": "mandate_activated", "scheme": "sepa_core" }
            }
        ]
    }"#;

    let env = GoCardlessWebhook::parse_envelope(body).expect("webhook envelope must parse");
    assert_eq!(env.events.len(), 2);

    let payment_ev = &env.events[0];
    assert_eq!(payment_ev.id, "EV0000000000A1");
    assert_eq!(payment_ev.resource_type, "payments");
    assert_eq!(payment_ev.action, "confirmed");
    assert_eq!(payment_ev.links.payment.as_deref(), Some("PM0000000001"));
    assert_eq!(
        payment_ev.details.as_ref().and_then(|d| d.cause.as_deref()),
        Some("payment_confirmed")
    );
    assert_eq!(
        payment_ev
            .metadata
            .get("meteroid.scenario")
            .map(String::as_str),
        Some("payment")
    );

    let mandate_ev = &env.events[1];
    assert_eq!(mandate_ev.resource_type, "mandates");
    assert_eq!(mandate_ev.action, "active");
    assert_eq!(mandate_ev.links.mandate.as_deref(), Some("MD0000000001"));

    eprintln!(
        "[d] parsed {} webhook events: {:?}",
        env.events.len(),
        env.events
            .iter()
            .map(|e| format!("{}.{}", e.resource_type, e.action))
            .collect::<Vec<_>>()
    );
}
