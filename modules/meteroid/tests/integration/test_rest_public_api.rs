//! Public REST API: checkout session redirect URLs, invoice refresh, subscription summary.

use serde_json::{Value, json};

use crate::data::ids::{CUST_UBER_ID, PLAN_LEETCODE_ID, PLAN_VERSION_1_LEETCODE_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::BaseId;
use meteroid_grpc::meteroid::api;

async fn json_or_panic(res: reqwest::Response) -> Value {
    let status = res.status();
    let body = res.text().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    serde_json::from_str(&body).unwrap()
}

/// Every response timestamp is RFC 3339 in UTC with millisecond precision, e.g.
/// `2026-10-30T09:12:33.123Z`.
fn assert_rfc3339(timestamp: &Value) {
    let text = timestamp.as_str().unwrap();
    let parsed = chrono::DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("not RFC 3339: {text}: {e}"));
    assert_eq!(
        text,
        parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "not UTC with millisecond precision"
    );
}

#[tokio::test]
async fn test_rest_public_api_gaps() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            api::apitokens::v1::CreateApiTokenRequest {
                name: "test-api-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key;
    let base = setup.config.rest_api_external_url.clone();
    let client = http_client_tracing::client();

    // Checkout session: merchant-set redirect URLs round-trip.
    let created: Value = client
        .post(format!("{base}/api/v1/checkout-sessions"))
        .bearer_auth(&api_key)
        .json(&json!({
            "customer_id": CUST_UBER_ID.as_base62(),
            "plan_version_id": PLAN_VERSION_1_LEETCODE_ID.as_base62(),
            "success_url": "https://acme.example/billing/done?src=meteroid",
            "cancel_url": "https://acme.example/pricing",
        }))
        .send()
        .await
        .map(json_or_panic)
        .unwrap()
        .await;
    let session = &created["session"];
    assert_eq!(
        session["success_url"],
        "https://acme.example/billing/done?src=meteroid"
    );
    assert_eq!(session["cancel_url"], "https://acme.example/pricing");
    assert!(session["checkout_url"].as_str().is_some());
    assert_rfc3339(&session["created_at"]);

    let fetched: Value = client
        .get(format!(
            "{base}/api/v1/checkout-sessions/{}",
            session["id"].as_str().unwrap()
        ))
        .bearer_auth(&api_key)
        .send()
        .await
        .map(json_or_panic)
        .unwrap()
        .await;
    assert_eq!(
        fetched["session"]["cancel_url"],
        "https://acme.example/pricing"
    );

    // Anything but an absolute http(s) URL is rejected: it is reflected to the browser.
    for bad in ["javascript:alert(1)", "/relative/path", "acme.example/done"] {
        let res = client
            .post(format!("{base}/api/v1/checkout-sessions"))
            .bearer_auth(&api_key)
            .json(&json!({
                "customer_id": CUST_UBER_ID.as_base62(),
                "plan_version_id": PLAN_VERSION_1_LEETCODE_ID.as_base62(),
                "success_url": bad,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST, "{bad}");
    }

    // Subscription with a draft invoice (auto-advance off keeps it a draft).
    let subscription: Value = client
        .post(format!("{base}/api/v1/subscriptions"))
        .bearer_auth(&api_key)
        .json(&json!({
            "plan_id": PLAN_LEETCODE_ID.as_base62(),
            "customer_id_or_alias": CUST_UBER_ID.as_base62(),
            "start_date": chrono::Utc::now().date_naive(),
            "activation_condition": "ON_START",
            "auto_advance_invoices": false,
            "charge_automatically": false,
        }))
        .send()
        .await
        .map(json_or_panic)
        .unwrap()
        .await;
    let subscription_id = subscription["id"].as_str().unwrap().to_string();

    // Lightweight read: the list-item shape, without components/add-ons/entitlements.
    let summary: Value = client
        .get(format!(
            "{base}/api/v1/subscriptions/{subscription_id}/summary"
        ))
        .bearer_auth(&api_key)
        .send()
        .await
        .map(json_or_panic)
        .unwrap()
        .await;
    assert_eq!(summary["id"], subscription_id);
    assert_eq!(summary["plan_id"], PLAN_LEETCODE_ID.as_base62());
    assert!(summary.get("components").is_none());
    assert!(summary.get("entitlements").is_none());
    // The spec says `date-time`, so typed SDKs parse timestamps as RFC 3339 with an offset.
    for timestamp in [&subscription["created_at"], &summary["created_at"]] {
        assert_rfc3339(timestamp);
    }

    let invoices: Value = client
        .get(format!("{base}/api/v1/invoices"))
        .bearer_auth(&api_key)
        .query(&[("subscription_id", subscription_id.as_str())])
        .send()
        .await
        .map(json_or_panic)
        .unwrap()
        .await;
    let draft = &invoices["data"][0];
    assert_eq!(draft["status"], "DRAFT");
    assert_rfc3339(&draft["created_at"]);
    let invoice_id = draft["id"].as_str().unwrap();

    let refreshed: Value = client
        .post(format!("{base}/api/v1/invoices/{invoice_id}/refresh"))
        .bearer_auth(&api_key)
        .send()
        .await
        .map(json_or_panic)
        .unwrap()
        .await;
    assert_eq!(refreshed["id"], invoice_id);
    assert_eq!(refreshed["status"], "DRAFT");
    assert_eq!(refreshed["total"], draft["total"]);
}

/// `coupon_code` is resolved when the session is created, exactly then ignoring the case of ASCII
/// letters: a code naming no live coupon, or several, is a 400 instead of a full-price checkout.
#[tokio::test]
async fn checkout_session_coupon_codes_are_checked_at_creation() {
    use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
    use meteroid_store::repositories::coupons::CouponInterface;

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            api::apitokens::v1::CreateApiTokenRequest {
                name: "test-api-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key;
    for code in ["Summer10", "SUMMER10", "Spring5"] {
        setup
            .store
            .create_coupon(
                common_domain::actor::Actor::System,
                CouponNew {
                    code: code.to_owned(),
                    description: code.to_owned(),
                    tenant_id: crate::data::ids::TENANT_ID,
                    discount: CouponDiscount::Percentage(rust_decimal_macros::dec!(10)),
                    expires_at: None,
                    redemption_limit: None,
                    recurring_value: None,
                    reusable: true,
                    plan_ids: vec![],
                },
            )
            .await
            .expect("coupon");
    }
    let base = setup.config.rest_api_external_url.clone();
    let client = http_client_tracing::client();
    let create = |code: Option<&str>| {
        client
            .post(format!("{base}/api/v1/checkout-sessions"))
            .bearer_auth(&api_key)
            .json(&json!({
                "customer_id": CUST_UBER_ID.as_base62(),
                "plan_version_id": PLAN_VERSION_1_LEETCODE_ID.as_base62(),
                "coupon_code": code,
            }))
            .send()
    };

    for (code, refusal) in [
        ("NOPE", "unknown coupon codes: NOPE"),
        (
            "summer10",
            "coupon codes matching several coupons that differ only in case, send the exact code: summer10",
        ),
    ] {
        let res = create(Some(code)).await.unwrap();
        assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST, "{code}");
        let body = res.text().await.unwrap();
        assert!(body.contains(refusal), "{body}");
    }

    for (code, stored) in [
        (Some("Summer10"), Some("Summer10")),
        (Some("spring5"), Some("spring5")),
        (Some("  "), None),
        (None, None),
    ] {
        let created: Value = json_or_panic(create(code).await.unwrap()).await;
        assert_eq!(
            created["session"]["coupon_code"].as_str(),
            stored,
            "{code:?}"
        );
    }
}
