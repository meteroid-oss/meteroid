//! Checkout sessions through the public REST API v2 (ADR-0013 §15): created by codes as a
//! subscription is (the customer by alias, the plan by code, its option, seats, add-ons and
//! coupons), replayed by their Idempotency-Key, read, listed by cursor, cancelled; snake_case
//! enums, prefixed ids, and the problems a refused request answers.
use crate::test_rest_v2::fixture;
use crate::test_rest_v2_subscriptions::seed;
use reqwest::StatusCode;
use serde_json::{Value, json};

fn ids(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Rules: ADR-0013 §15
#[tokio::test]
async fn test_rest_v2_checkout_sessions() {
    let f = fixture().await;
    seed(&f).await;

    // Created by codes, with its hosted page.
    let body = json!({
        "customer": "ext:spotify",
        "plan": "studio",
        "option": "annual",
        "quantities": {"users": "4"},
        "add_ons": [{"add_on": "support"}],
        "coupons": ["launch10"],
        "trial_days": 14,
        "invoicing": {"net_terms": 30, "memo": "Thanks"},
        "success_url": "https://acme.example/billing/done",
        "cancel_url": "https://acme.example/pricing",
        "metadata": {"crm": "42"}
    });
    let created = f
        .post_once("/api/v2/checkout-sessions", "checkout-studio", body.clone())
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let session = &created.body;
    let id = session["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("che_"), "{id}");
    assert_eq!(session["type"], "self_serve");
    assert_eq!(session["status"], "created");
    // What it names reads as references: an id and the human key.
    assert!(
        session["customer"]["id"]
            .as_str()
            .unwrap()
            .starts_with("cus_")
    );
    assert_eq!(session["customer"]["alias"], "spotify");
    assert!(session["plan"]["id"].as_str().unwrap().starts_with("plan_"));
    assert_eq!(
        (
            &session["plan"]["code"],
            &session["plan"]["version"],
            &session["plan"]["option"]
        ),
        (&json!("studio"), &json!(1), &json!("annual"))
    );
    assert_eq!(session["add_ons"][0]["add_on"]["code"], "support");
    assert!(
        session["add_ons"][0]["add_on"]["id"]
            .as_str()
            .unwrap()
            .starts_with("add_")
    );
    assert_eq!(session["add_ons"][0]["quantity"], 1);
    assert_eq!(session["coupons"][0]["code"], "LAUNCH10");
    assert!(
        session["coupons"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("cou_")
    );
    assert_eq!(session["trial_days"], 14);
    assert_eq!(session["invoicing"]["net_terms"], 30);
    assert_eq!(session["invoicing"]["memo"], "Thanks");
    assert_eq!(session["invoicing"]["auto_finalize"], true);
    assert_eq!(session["success_url"], "https://acme.example/billing/done");
    assert_eq!(session["metadata"], json!({"crm": "42"}));
    assert!(
        session["url"]
            .as_str()
            .unwrap()
            .contains("/checkout?token="),
        "{session:#}"
    );
    assert!(session["expires_at"].is_string(), "an hour by default");
    assert!(session.get("checkout_type").is_none() && session.get("plan_version_id").is_none());

    // The same key and body replays it; another body under the key is refused.
    let replay = f
        .post_once("/api/v2/checkout-sessions", "checkout-studio", body)
        .await;
    assert_eq!(replay.status, StatusCode::CREATED);
    assert_eq!(replay.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replay.body["id"], id.as_str());
    let reused = f
        .post_once(
            "/api/v2/checkout-sessions",
            "checkout-studio",
            json!({"customer": "ext:spotify", "plan": "scale"}),
        )
        .await;
    assert_eq!(reused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reused.body["code"], "idempotency_key_reused");

    // Read by id.
    let read = f.get(&format!("/api/v2/checkout-sessions/{id}")).await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["plan"]["option"], "annual");
    assert!(read.body["url"].is_string());

    // Refused at their JSON pointer.
    let refusals = [
        (
            json!({"customer": "ext:nobody", "plan": "studio"}),
            "/customer",
        ),
        (
            json!({"customer": "ext:spotify", "plan": "no-such-plan"}),
            "/plan",
        ),
        (
            json!({"customer": "ext:spotify", "plan": "studio", "option": "weekly"}),
            "/option",
        ),
        (
            json!({"customer": "ext:spotify", "plan": "studio", "quantities": {"licences": "2"}}),
            "/quantities/licences",
        ),
        (
            json!({"customer": "ext:spotify", "plan": "studio", "success_url": "javascript:alert(1)"}),
            "/success_url",
        ),
    ];
    for (body, path) in refusals {
        let refused = f.post("/api/v2/checkout-sessions", body).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}: {:#}",
            refused.body
        );
        assert_eq!(
            refused.header("content-type"),
            Some("application/problem+json")
        );
        assert_eq!(
            refused.body["errors"][0]["path"], path,
            "{:#}",
            refused.body
        );
    }

    // Listed by cursor, in creation order, without their page.
    let other = f
        .post(
            "/api/v2/checkout-sessions",
            json!({"customer": "ext:spotify", "plan": "scale", "expires_in_hours": 0}),
        )
        .await;
    assert_eq!(other.status, StatusCode::CREATED, "{:#}", other.body);
    let other_id = other.body["id"].as_str().unwrap().to_owned();
    assert!(other.body.get("expires_at").is_none(), "never expires");
    let first = f
        .get("/api/v2/checkout-sessions?customer=ext:spotify&limit=1")
        .await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(ids(&first.body), vec![id.clone()]);
    assert_eq!(first.body["has_more"], true);
    assert!(first.body["data"][0].get("url").is_none());
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f
        .get(&format!(
            "/api/v2/checkout-sessions?customer=ext:spotify&limit=1&after={cursor}"
        ))
        .await;
    assert_eq!(ids(&second.body), vec![other_id.clone()]);
    assert_eq!(second.body["has_more"], false);
    // A query value that does not read is refused at its parameter, as a bad limit is.
    let shouting = f.get("/api/v2/checkout-sessions?status=CREATED").await;
    assert_eq!(shouting.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(shouting.body["code"], "validation_failed");
    assert_eq!(shouting.body["errors"][0]["path"], "status");

    // Cancelled once; a second time is a conflict.
    let cancelled = f
        .post(
            &format!("/api/v2/checkout-sessions/{other_id}/cancel"),
            json!({}),
        )
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:#}", cancelled.body);
    assert_eq!(cancelled.body["status"], "cancelled");
    assert!(cancelled.body.get("url").is_none());
    let again = f
        .post(
            &format!("/api/v2/checkout-sessions/{other_id}/cancel"),
            json!({}),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{:#}", again.body);
    assert_eq!(again.body["code"], "conflict");
    let cancelled_only = f
        .get("/api/v2/checkout-sessions?customer=ext:spotify&status=cancelled")
        .await;
    assert_eq!(ids(&cancelled_only.body), vec![other_id]);

    let missing = f.get("/api/v2/checkout-sessions/che_nonexistent").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.body["code"], "not_found");
}
