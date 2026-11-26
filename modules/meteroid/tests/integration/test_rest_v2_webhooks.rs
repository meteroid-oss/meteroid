//! Webhook endpoints through the public REST API v2 (ADR-0013 §13): created once per
//! Idempotency-Key (the secret never replayed), read, listed by cursor, patched (RFC 7396),
//! archived, their secret read and rotated, their deliveries listed newest first by cursor and
//! resent; snake_case enums, prefixed ids, event types as a payload's `type` names them, and the
//! problems a refused request answers.
use crate::data::ids::TENANT_ID;
use crate::test_rest_v2::{Answer, Fixture, fixture, second_tenant_key};
use common_domain::ids::{BaseId, EventId, WebhookDeliveryId, WebhookEndpointId};
use meteroid_store::domain::webhooks_out::WebhookOutMessageNew;
use meteroid_store::domain::webhooks_out::WebhookOutTarget;
use meteroid_store::repositories::WebhookOutInterface;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::str::FromStr;

async fn delete(f: &Fixture, path: &str) -> Answer {
    f.send(f.client.delete(f.url(path)).bearer_auth(&f.api_key))
        .await
}

fn ids(page: &Value) -> Vec<String> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Rules: ADR-0013 §13
#[tokio::test]
async fn test_rest_v2_webhook_endpoints() {
    let f = fixture().await;

    // Created with its secret, its event types as payloads name them, a sensitive header
    // write-only.
    let body = json!({
        "url": "https://hooks.example/meteroid",
        "description": "Billing events",
        "event_types": ["invoice.paid", "subscription.created"],
        "headers": [
            {"name": "X-Team", "value": "billing"},
            {"name": "Authorization", "value": "Bearer s3cr3t", "sensitive": true}
        ],
        "rate_limit_per_sec": 10
    });
    let created = f
        .post_once("/api/v2/webhooks/endpoints", "create-hook", body.clone())
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let endpoint = &created.body;
    let id = endpoint["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("whe_"), "{id}");
    let secret = endpoint["secret"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("whsec_"), "{secret}");
    assert_eq!(
        endpoint["event_types"],
        json!(["invoice.paid", "subscription.created"])
    );
    assert_eq!(endpoint["disabled"], false);
    assert_eq!(endpoint["needs_setup"], false);
    assert_eq!(endpoint["rate_limit_per_sec"], 10);
    assert_eq!(
        endpoint["api_version"],
        meteroid_rest::v2::version::latest_version().as_header(),
        "the latest version when the request names none"
    );
    // A version asked is the one pinned.
    let pinned = f
        .send(
            f.client
                .post(f.url("/api/v2/webhooks/endpoints"))
                .bearer_auth(&f.api_key)
                .header("Idempotency-Key", "create-pinned-hook")
                .header("Meteroid-Version", "2026-12-01")
                .json(&json!({"url": "https://hooks.example/pinned"})),
        )
        .await;
    assert_eq!(pinned.status, StatusCode::CREATED, "{:#}", pinned.body);
    assert_eq!(pinned.body["api_version"], "2026-12-01");
    let archived = f
        .post(
            &format!(
                "/api/v2/webhooks/endpoints/{}/archive",
                pinned.body["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    let authorization = endpoint["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|header| header["name"] == "Authorization")
        .unwrap();
    assert_eq!(authorization["sensitive"], true);
    assert_eq!(authorization["set"], true);
    assert!(authorization.get("value").is_none(), "{authorization:#}");

    // A retry replays the creation, never its secret: it is read on its own.
    let replay = f
        .post_once("/api/v2/webhooks/endpoints", "create-hook", body)
        .await;
    assert_eq!(replay.status, StatusCode::CREATED);
    assert_eq!(replay.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replay.body["idempotent_replay"], "response_not_kept");
    assert_eq!(
        replay.body["id"],
        id.as_str(),
        "the marker names what was created"
    );
    assert!(replay.body.get("secret").is_none(), "{:#}", replay.body);
    let read_secret = f
        .get(&format!("/api/v2/webhooks/endpoints/{id}/secret"))
        .await;
    assert_eq!(read_secret.status, StatusCode::OK, "{:#}", read_secret.body);
    assert_eq!(read_secret.body["secret"], secret.as_str());

    // Refused at its JSON pointer: an event type no payload carries (v1's enum spelling
    // included), a destination that is not one.
    let unknown = f
        .post(
            "/api/v2/webhooks/endpoints",
            json!({"url": "https://hooks.example/x", "event_types": ["invoice.paid", "INVOICE_PAID"]}),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unknown.header("content-type"),
        Some("application/problem+json")
    );
    assert_eq!(unknown.body["code"], "validation_failed");
    assert_eq!(unknown.body["errors"][0]["path"], "/event_types/1");
    let nowhere = f
        .post("/api/v2/webhooks/endpoints", json!({"url": "not a url"}))
        .await;
    assert_eq!(nowhere.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(nowhere.body["errors"][0]["path"], "/url");

    // Read by its id; listed by cursor in creation order.
    let read = f.get(&format!("/api/v2/webhooks/endpoints/{id}")).await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["url"], "https://hooks.example/meteroid");
    assert!(read.body.get("secret").is_none());
    let other = f
        .post(
            "/api/v2/webhooks/endpoints",
            json!({"url": "https://other.example/hook"}),
        )
        .await;
    assert_eq!(other.status, StatusCode::CREATED, "{:#}", other.body);
    let other_id = other.body["id"].as_str().unwrap().to_owned();
    assert_eq!(other.body["event_types"], json!([]));
    let first = f.get("/api/v2/webhooks/endpoints?limit=1").await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(ids(&first.body), vec![id.clone()]);
    assert_eq!(first.body["has_more"], true);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f
        .get(&format!(
            "/api/v2/webhooks/endpoints?limit=1&after={cursor}"
        ))
        .await;
    assert_eq!(ids(&second.body), vec![other_id.clone()]);
    assert_eq!(second.body["has_more"], false);
    assert!(second.body["next_cursor"].is_null());

    // A merge patch: `null` clears, a list replaces, what is left out stays.
    let patched = f
        .patch(
            &format!("/api/v2/webhooks/endpoints/{id}"),
            json!({"description": null, "event_types": ["credit_note.created"], "rate_limit_per_sec": null}),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert!(
        patched.body.get("description").is_none(),
        "{:#}",
        patched.body
    );
    assert!(patched.body.get("rate_limit_per_sec").is_none());
    assert_eq!(patched.body["event_types"], json!(["credit_note.created"]));
    assert_eq!(patched.body["url"], "https://hooks.example/meteroid");
    let refused = f
        .patch(
            &format!("/api/v2/webhooks/endpoints/{id}"),
            json!({"event_types": ["invoice.exploded"]}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["errors"][0]["path"], "/event_types/0");

    // Its API version changes to a published one (v2's payloads), and is never removed.
    let pinned = f
        .patch(
            &format!("/api/v2/webhooks/endpoints/{id}"),
            json!({"api_version": "2026-12-15"}),
        )
        .await;
    assert_eq!(pinned.status, StatusCode::OK, "{:#}", pinned.body);
    assert_eq!(pinned.body["api_version"], "2026-12-15");
    for asked in [json!("2025-01-01"), json!("2027-01-01"), Value::Null] {
        let refused = f
            .patch(
                &format!("/api/v2/webhooks/endpoints/{id}"),
                json!({"api_version": asked}),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{asked}: {:#}",
            refused.body
        );
        assert_eq!(refused.body["errors"][0]["path"], "/api_version");
    }

    // An endpoint created through v1 has none: v1's payloads.
    let through_v1 = f
        .send(
            f.client
                .post(f.url("/api/v1/webhooks/endpoints"))
                .bearer_auth(&f.api_key)
                .json(&json!({"url": "https://v1.example/hook"})),
        )
        .await;
    assert!(
        through_v1.status.is_success(),
        "{}: {:#}",
        through_v1.status,
        through_v1.body
    );
    let v1_id = through_v1.body["id"].as_str().unwrap().to_owned();
    let v1_read = f.get(&format!("/api/v2/webhooks/endpoints/{v1_id}")).await;
    assert_eq!(v1_read.status, StatusCode::OK, "{:#}", v1_read.body);
    assert!(
        v1_read.body.get("api_version").is_none(),
        "{:#}",
        v1_read.body
    );
    assert_eq!(
        f.post(
            &format!("/api/v2/webhooks/endpoints/{v1_id}/archive"),
            json!({})
        )
        .await
        .status,
        StatusCode::OK
    );

    // Rotated: the new secret is the one read.
    let rotated = f
        .post_once(
            &format!("/api/v2/webhooks/endpoints/{id}/rotate-secret"),
            "rotate-hook",
            json!({}),
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{:#}", rotated.body);
    let new_secret = rotated.body["secret"].as_str().unwrap().to_owned();
    assert_ne!(new_secret, secret);
    let read_again = f
        .get(&format!("/api/v2/webhooks/endpoints/{id}/secret"))
        .await;
    assert_eq!(read_again.body["secret"], new_secret.as_str());

    // Archived: answered with its `archived_at`, no longer read, archived once; no DELETE.
    let archived = f
        .post(
            &format!("/api/v2/webhooks/endpoints/{other_id}/archive"),
            json!({}),
        )
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["id"], other_id.as_str());
    assert!(
        archived.body["archived_at"].is_string(),
        "{:#}",
        archived.body
    );
    let again = f
        .post(
            &format!("/api/v2/webhooks/endpoints/{other_id}/archive"),
            json!({}),
        )
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    let no_delete = delete(&f, &format!("/api/v2/webhooks/endpoints/{id}")).await;
    assert_eq!(
        no_delete.status,
        StatusCode::METHOD_NOT_ALLOWED,
        "{:#}",
        no_delete.body
    );
    let gone = f
        .get(&format!("/api/v2/webhooks/endpoints/{other_id}"))
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.body["code"], "not_found");
    let not_an_id = f.get("/api/v2/webhooks/endpoints/whatever").await;
    assert_eq!(not_an_id.status, StatusCode::NOT_FOUND);
}

/// Rules: ADR-0013 §13
#[tokio::test]
async fn test_rest_v2_webhook_deliveries() {
    let f = fixture().await;
    let created = f
        .post(
            "/api/v2/webhooks/endpoints",
            json!({"url": "https://deliveries.example/hook"}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let id = created.body["id"].as_str().unwrap().to_owned();
    let endpoint_id = WebhookEndpointId::from_str(&id).unwrap();

    for index in 0..3 {
        f.setup
            .store
            .insert_webhook_message_with_deliveries(
                WebhookOutMessageNew {
                    id: EventId::new(),
                    tenant_id: TENANT_ID,
                    event_type: "customer.created".to_owned(),
                    payload: json!({"index": index}),
                },
                vec![],
                &[WebhookOutTarget {
                    endpoint_id,
                    api_version: None,
                }],
            )
            .await
            .unwrap();
    }

    // Newest first, by cursor.
    let first = f
        .get(&format!(
            "/api/v2/webhooks/endpoints/{id}/deliveries?limit=2"
        ))
        .await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    let page = first.body["data"].as_array().unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(first.body["has_more"], true);
    let delivery = &page[0];
    assert!(delivery["id"].as_str().unwrap().starts_with("whd_"));
    assert!(delivery["event"].as_str().unwrap().starts_with("evt_"));
    assert_eq!(delivery["endpoint"], id.as_str());
    assert_eq!(delivery["event_type"], "customer.created");
    let status = delivery["status"].as_str().unwrap();
    assert!(
        ["pending", "in_flight", "succeeded", "failed", "cancelled"].contains(&status),
        "{status}"
    );
    assert!(page[0]["created_at"].as_str().unwrap() >= page[1]["created_at"].as_str().unwrap());
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f
        .get(&format!(
            "/api/v2/webhooks/endpoints/{id}/deliveries?limit=2&after={cursor}"
        ))
        .await;
    assert_eq!(second.status, StatusCode::OK, "{:#}", second.body);
    assert_eq!(second.body["data"].as_array().unwrap().len(), 1);
    assert_eq!(second.body["has_more"], false);
    let mut all = ids(&first.body);
    all.extend(ids(&second.body));
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 3);

    // One delivery, as the list reads it; another tenant's key reads none.
    let delivery_id = delivery["id"].as_str().unwrap().to_owned();
    let one = f
        .get(&format!("/api/v2/webhooks/deliveries/{delivery_id}"))
        .await;
    assert_eq!(one.status, StatusCode::OK, "{:#}", one.body);
    for field in [
        "id",
        "event",
        "endpoint",
        "event_type",
        "manual",
        "created_at",
    ] {
        assert_eq!(one.body[field], delivery[field], "{field}");
    }
    for missing in [
        "/api/v2/webhooks/deliveries/whd_nonexistent".to_owned(),
        format!("/api/v2/webhooks/deliveries/{}", WebhookDeliveryId::new()),
    ] {
        let answer = f.get(&missing).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{missing}");
        assert_eq!(answer.body["code"], "not_found");
    }
    let other_key = second_tenant_key(&f).await;
    let theirs = f
        .send(
            f.client
                .get(f.url(&format!("/api/v2/webhooks/deliveries/{delivery_id}")))
                .bearer_auth(&other_key),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND, "{:#}", theirs.body);

    // Several states, the parameter repeated: every state names all three.
    let every_state = f
        .get(&format!(
            "/api/v2/webhooks/endpoints/{id}/deliveries?status=pending&status=in_flight\
             &status=succeeded&status=failed&status=cancelled&limit=100"
        ))
        .await;
    assert_eq!(every_state.status, StatusCode::OK, "{:#}", every_state.body);
    assert_eq!(ids(&every_state.body).len(), 3);
    // A cursor that is none of the endpoint's deliveries (purged since, or forged) is refused:
    // without its `created_at` no page from it would be exact.
    let unknown = meteroid_rest::v2::pagination::Cursor(uuid::Uuid::now_v7()).encode();
    let refused_cursor = f
        .get(&format!(
            "/api/v2/webhooks/endpoints/{id}/deliveries?limit=2&after={unknown}"
        ))
        .await;
    assert_eq!(
        refused_cursor.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused_cursor.body
    );
    assert_eq!(refused_cursor.body["errors"][0]["path"], "after");

    // Disabled, its pending deliveries are cancelled, and a resend is refused.
    let disabled = f
        .patch(
            &format!("/api/v2/webhooks/endpoints/{id}"),
            json!({"disabled": true}),
        )
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{:#}", disabled.body);
    assert_eq!(disabled.body["disabled_reason"], "manual");
    let cancelled = f
        .get(&format!(
            "/api/v2/webhooks/endpoints/{id}/deliveries?status=cancelled&limit=100"
        ))
        .await;
    assert_eq!(cancelled.status, StatusCode::OK, "{:#}", cancelled.body);
    for delivery in cancelled.body["data"].as_array().unwrap() {
        assert_eq!(delivery["status"], "cancelled");
    }
    let delivery_id = page[0]["id"].as_str().unwrap().to_owned();
    let refused = f
        .post(
            &format!("/api/v2/webhooks/deliveries/{delivery_id}/resend"),
            json!({}),
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{:#}", refused.body);
    assert_eq!(refused.body["code"], "conflict");

    // Enabled again, the delivery is queued again (unless the worker is sending it right now).
    let enabled = f
        .patch(
            &format!("/api/v2/webhooks/endpoints/{id}"),
            json!({"disabled": false}),
        )
        .await;
    assert_eq!(enabled.status, StatusCode::OK, "{:#}", enabled.body);
    assert!(enabled.body.get("disabled_reason").is_none());
    let resent = f
        .post(
            &format!("/api/v2/webhooks/deliveries/{delivery_id}/resend"),
            json!({}),
        )
        .await;
    match resent.status {
        StatusCode::OK => {
            assert_eq!(resent.body["id"], delivery_id.as_str());
            assert_eq!(resent.body["manual"], true);
        }
        StatusCode::CONFLICT => assert_eq!(resent.body["code"], "conflict"),
        other => panic!("resend answered {other}: {:#}", resent.body),
    }

    // A status that is none is unreadable; a delivery that is none is not found.
    let unreadable = f
        .get(&format!(
            "/api/v2/webhooks/endpoints/{id}/deliveries?status=PENDING"
        ))
        .await;
    assert_eq!(unreadable.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unreadable.body["code"], "validation_failed");
    assert_eq!(unreadable.body["errors"][0]["path"], "status");
    let missing = f
        .post(
            "/api/v2/webhooks/deliveries/whd_nonexistent/resend",
            json!({}),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}
