//! What API v2 refuses before the store (ADR-0013 §3, §5): every price written is checked the
//! same way wherever it is written (a currency Meteroid knows; no negative amount but a per-unit
//! rate on usage, a reversal, R-142), an option
//! bills a charge on one cadence per currency, inputs stay within their bounds; each refusal a
//! `validation_failed` at its JSON pointer. Also: archived plans left out of lists, metrics
//! sharing an event code whatever API writes them, a checkout session read whatever became of
//! its coupons, a customer search matched literally. A body that does not read (a field
//! it does not have, of the wrong type, missing) is `invalid_request` at the field's pointer.
use crate::data::ids::{PRODUCT_FAMILY_ID, TENANT_ID};
use crate::test_rest_v2::{Answer, Fixture, fixture};
use crate::test_rest_v2_subscriptions::seed;
use common_domain::ids::CouponId;
use meteroid_store::repositories::coupons::CouponInterface;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::str::FromStr;

/// A `validation_failed` problem whose first error is at `path`.
#[track_caller]
fn refused_at(answer: &Answer, path: &str) {
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{path}: {:#}",
        answer.body
    );
    assert_eq!(
        answer.header("content-type"),
        Some("application/problem+json")
    );
    assert_eq!(
        answer.body["code"], "validation_failed",
        "{:#}",
        answer.body
    );
    assert_eq!(answer.body["errors"][0]["path"], path, "{:#}", answer.body);
}

/// An `invalid_request` problem whose first error, `code`, is at `path`.
#[track_caller]
fn unreadable_at(answer: &Answer, path: &str, code: &str) {
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "{path}: {:#}",
        answer.body
    );
    assert_eq!(answer.body["code"], "invalid_request", "{:#}", answer.body);
    assert_eq!(answer.body["errors"][0]["path"], path, "{:#}", answer.body);
    assert_eq!(answer.body["errors"][0]["code"], code, "{:#}", answer.body);
}

/// A plan of one charge, `charge`, in the options `options` (aliases).
fn plan_of(code: &str, options: &[&str], charge: Value) -> Value {
    json!({
        "code": code,
        "name": code,
        "draft": {
            "options": options
                .iter()
                .map(|alias| json!({"alias": alias, "name": alias}))
                .collect::<Vec<_>>(),
            "charges": [charge]
        }
    })
}

fn flat(price: Value) -> Value {
    json!({"product": "platform", "preset": "flat_fee", "prices": [price]})
}

async fn subscribe(f: &Fixture, key: &str, extra: Value) -> Answer {
    let mut body = json!({
        "customer": "ext:spotify",
        "plan": "studio",
        "option": "monthly",
        "quantities": {"users": "3"}
    });
    for (field, value) in extra.as_object().unwrap() {
        body[field] = value.clone();
    }
    f.post_once("/api/v2/subscriptions", key, body).await
}

/// Rules: ADR-0013 §3, §5
#[tokio::test]
async fn test_rest_v2_prices_and_bounds_are_checked_at_their_pointer() {
    let f = fixture().await;
    seed(&f).await;

    // A price's currency is one Meteroid knows, wherever it is written.
    let unknown_currency = f
        .post(
            "/api/v2/plans",
            plan_of(
                "euro",
                &[],
                flat(json!({"currency": "EURO", "model": {"type": "flat", "amount": "49"}})),
            ),
        )
        .await;
    refused_at(&unknown_currency, "/draft/charges/0/prices/0/currency");
    let mut version_currency = plan_of(
        "zzz",
        &[],
        flat(json!({"currency": "EUR", "model": {"type": "flat", "amount": "49"}})),
    );
    version_currency["draft"]["currency"] = json!("ZZZ");
    refused_at(
        &f.post("/api/v2/plans", version_currency).await,
        "/draft/currency",
    );
    // A negative per-unit rate on usage is a reversal (R-142): a commission given back per unit
    // routed, written as it is.
    let reversal = f
        .post(
            "/api/v2/plans",
            plan_of(
                "reversal",
                &[],
                json!({
                    "product": "api",
                    "quantity": {"type": "metered", "metric": "bandwidth"},
                    "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "-0.01"}}]
                }),
            ),
        )
        .await;
    assert_eq!(reversal.status, StatusCode::CREATED, "{:#}", reversal.body);
    let rebate = f
        .post(
            "/api/v2/add-ons",
            json!({
                "code": "rebate",
                "name": "Rebate",
                "charge": {
                    "product": "support",
                    "preset": "flat_fee",
                    "prices": [{"currency": "EUR", "model": {"type": "flat", "amount": "-29"}}]
                }
            }),
        )
        .await;
    // Any other negative price is refused: the pricer would bill it at 0 (R-142).
    refused_at(&rebate, "/charge/prices/0/model/amount");
    // Zero is a free price.
    let free = f
        .post(
            "/api/v2/plans",
            plan_of(
                "zero-price",
                &[],
                flat(json!({"currency": "EUR", "model": {"type": "flat", "amount": "0"}})),
            ),
        )
        .await;
    assert_eq!(free.status, StatusCode::CREATED, "{:#}", free.body);

    // An option is one way of buying the plan: it bills a charge on one cadence per currency.
    let two_cadences = f
        .post(
            "/api/v2/plans",
            plan_of(
                "cadences",
                &["monthly", "annual"],
                json!({"product": "platform", "preset": "flat_fee", "prices": [
                    {"option": "*", "currency": "EUR", "cadence": "monthly",
                     "model": {"type": "flat", "amount": "49"}},
                    {"option": "annual", "currency": "EUR", "cadence": "annual",
                     "model": {"type": "flat", "amount": "490"}}
                ]}),
            ),
        )
        .await;
    refused_at(&two_cadences, "/draft/charges/0/prices/1/cadence");
    assert!(
        two_cadences.body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("one cadence per currency"),
        "{:#}",
        two_cadences.body
    );
    let one_option_two_cadences = f
        .post(
            "/api/v2/plans",
            plan_of(
                "cadences-one",
                &[],
                json!({"product": "platform", "preset": "flat_fee", "prices": [
                    {"currency": "EUR", "cadence": "monthly", "model": {"type": "flat", "amount": "49"}},
                    {"currency": "EUR", "cadence": "annual", "model": {"type": "flat", "amount": "490"}}
                ]}),
            ),
        )
        .await;
    refused_at(
        &one_option_two_cadences,
        "/draft/charges/0/prices/1/cadence",
    );

    // Bounds, each at its pointer.
    let mut late_terms = plan_of(
        "terms",
        &[],
        flat(json!({"currency": "EUR", "model": {"type": "flat", "amount": "49"}})),
    );
    late_terms["draft"]["net_terms"] = json!(400);
    refused_at(
        &f.post("/api/v2/plans", late_terms).await,
        "/draft/net_terms",
    );
    let trialed = plan_of(
        "trial",
        &[],
        flat(json!({"currency": "EUR", "model": {"type": "flat", "amount": "49"}})),
    );
    let created = f.post("/api/v2/plans", trialed).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    refused_at(
        &f.patch(
            "/api/v2/plans/trial",
            json!({"trial": {"type": "terms", "days": 100000}}),
        )
        .await,
        "/trial/days",
    );
    let many_charges: Vec<Value> = (0..101)
        .map(|index| json!({"product": "platform", "name": format!("Fee {index}")}))
        .collect();
    refused_at(
        &f.post(
            "/api/v2/plans",
            json!({"code": "many", "name": "Many", "draft": {"charges": many_charges}}),
        )
        .await,
        "/draft/charges",
    );
    refused_at(
        &f.post(
            "/api/v2/plans",
            json!({"code": "long", "name": "x".repeat(256), "draft": {}}),
        )
        .await,
        "/name",
    );
    refused_at(
        &f.post(
            "/api/v2/products",
            json!({"code": "wordy", "name": "Wordy", "description": "x".repeat(4097)}),
        )
        .await,
        "/description",
    );
    refused_at(
        &subscribe(&f, "anchor", json!({"billing_day_anchor": 0})).await,
        "/billing_day_anchor",
    );
    refused_at(
        &subscribe(&f, "anchor-32", json!({"billing_day_anchor": 32})).await,
        "/billing_day_anchor",
    );
    refused_at(
        &subscribe(
            &f,
            "trial",
            json!({"trial": {"type": "custom", "days": 100000}}),
        )
        .await,
        "/trial/days",
    );
    // Never wrapped: 4294967295 days of net terms would be stored as -1.
    refused_at(
        &subscribe(
            &f,
            "net-terms",
            json!({"invoicing": {"net_terms": 4294967295u32}}),
        )
        .await,
        "/invoicing/net_terms",
    );
    refused_at(
        &subscribe(
            &f,
            "add-on-quantity",
            json!({"add_ons": [{"add_on": "support", "quantity": 3000000000u32}]}),
        )
        .await,
        "/add_ons/0/quantity",
    );
    refused_at(
        &subscribe(&f, "seats", json!({"quantities": {"users": "3000000000"}})).await,
        "/quantities/users",
    );
}

/// Rules: ADR-0013 §3, §6
#[tokio::test]
async fn test_rest_v2_archived_plans_metric_codes_checkout_coupons_and_search() {
    let f = fixture().await;
    seed(&f).await;

    // A plan list leaves archived plans out unless asked, as every list does.
    let retired = f
        .post(
            "/api/v2/plans",
            plan_of(
                "retired",
                &[],
                flat(json!({"currency": "EUR", "model": {"type": "flat", "amount": "9"}})),
            ),
        )
        .await;
    assert_eq!(retired.status, StatusCode::CREATED, "{:#}", retired.body);
    let archived = f.post("/api/v2/plans/retired/archive", json!({})).await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    let codes = |page: &Value| -> Vec<String> {
        page["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|plan| plan["code"].as_str().unwrap().to_owned())
            .collect()
    };
    let listed = f.get("/api/v2/plans?limit=100").await;
    assert_eq!(listed.status, StatusCode::OK, "{:#}", listed.body);
    assert!(codes(&listed.body).contains(&"studio".to_owned()));
    assert!(
        !codes(&listed.body).contains(&"retired".to_owned()),
        "{:#}",
        listed.body
    );
    let asked = f.get("/api/v2/plans?status=archived&limit=100").await;
    assert_eq!(codes(&asked.body), vec!["retired".to_owned()]);
    // A status that does not read is refused at its parameter.
    let shouting = f.get("/api/v2/plans?status=ARCHIVED").await;
    refused_at(&shouting, "status");

    // Metrics of one event code, whatever API creates them: v1's gets its own code, the event
    // code numbered, both live and each read by its code.
    let created = f
        .post(
            "/api/v2/metrics",
            json!({"code": "dup_calls", "name": "Calls", "aggregation": "count"}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let twin = f
        .send(
            f.client
                .post(f.url("/api/v1/metrics"))
                .bearer_auth(&f.api_key)
                .json(&json!({
                    "name": "Twin",
                    "code": "dup_calls",
                    "aggregation_type": "COUNT",
                    "product_family_id": PRODUCT_FAMILY_ID.to_string(),
                })),
        )
        .await;
    assert_eq!(twin.status, StatusCode::CREATED, "{:#}", twin.body);
    let read_twin = f.get("/api/v2/metrics/dup_calls-2").await;
    assert_eq!(read_twin.status, StatusCode::OK, "{:#}", read_twin.body);
    assert_eq!(read_twin.body["id"], twin.body["id"]);
    assert_eq!(read_twin.body["event_code"], "dup_calls");
    assert_eq!(read_twin.body["archived"], false);
    let read_first = f.get("/api/v2/metrics/dup_calls").await;
    assert_eq!(read_first.body["id"], created.body["id"]);
    assert_eq!(read_first.body["archived"], false);

    // A checkout session expires within a year, or never.
    let session = |extra: Value| {
        let mut body = json!({"customer": "ext:spotify", "plan": "studio", "option": "monthly"});
        for (field, value) in extra.as_object().unwrap() {
            body[field] = value.clone();
        }
        body
    };
    refused_at(
        &f.post(
            "/api/v2/checkout-sessions",
            session(json!({"expires_in_hours": 9000})),
        )
        .await,
        "/expires_in_hours",
    );
    refused_at(
        &f.post(
            "/api/v2/checkout-sessions",
            session(json!({"expires_in_hours": 4294967295u32})),
        )
        .await,
        "/expires_in_hours",
    );
    refused_at(
        &f.post(
            "/api/v2/checkout-sessions",
            session(json!({"billing_day_anchor": 0})),
        )
        .await,
        "/billing_day_anchor",
    );
    let never = f
        .post(
            "/api/v2/checkout-sessions",
            session(json!({"expires_in_hours": 0})),
        )
        .await;
    assert_eq!(never.status, StatusCode::CREATED, "{:#}", never.body);
    assert!(never.body["expires_at"].is_null(), "{:#}", never.body);

    // A coupon deleted since a session named it is left out of the session, never a 404.
    let coupon = f
        .post(
            "/api/v2/coupons",
            json!({"code": "GONE5", "discount": {"type": "percentage", "percentage": "5"}}),
        )
        .await;
    assert_eq!(coupon.status, StatusCode::CREATED, "{:#}", coupon.body);
    let with_coupon = f
        .post(
            "/api/v2/checkout-sessions",
            session(json!({"coupons": ["GONE5"]})),
        )
        .await;
    assert_eq!(
        with_coupon.status,
        StatusCode::CREATED,
        "{:#}",
        with_coupon.body
    );
    assert_eq!(
        with_coupon.body["coupons"],
        json!([{"id": coupon.body["id"], "code": "GONE5"}])
    );
    let coupon_id = CouponId::from_str(coupon.body["id"].as_str().unwrap()).unwrap();
    f.setup
        .store
        .delete_coupon(TENANT_ID, coupon_id)
        .await
        .unwrap();
    let session_id = with_coupon.body["id"].as_str().unwrap();
    let read = f
        .get(&format!("/api/v2/checkout-sessions/{session_id}"))
        .await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["coupons"], json!([]));
    let listed = f.get("/api/v2/checkout-sessions?limit=100").await;
    assert_eq!(listed.status, StatusCode::OK, "{:#}", listed.body);
    assert!(
        listed.body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed["id"] == session_id),
        "{:#}",
        listed.body
    );

    // A customer search is matched literally: `%` and `_` are no wildcards.
    let aliases = |page: &Value| -> Vec<String> {
        page["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|customer| customer["alias"].as_str().map(str::to_owned))
            .collect()
    };
    let found = f.get("/api/v2/customers?search=potif").await;
    assert_eq!(found.status, StatusCode::OK, "{:#}", found.body);
    assert_eq!(aliases(&found.body), vec!["spotify".to_owned()]);
    for wildcard in ["%25", "_", "%5C"] {
        let none = f.get(&format!("/api/v2/customers?search={wildcard}")).await;
        assert_eq!(none.status, StatusCode::OK, "{:#}", none.body);
        assert_eq!(
            none.body["data"],
            json!([]),
            "{wildcard} matched as a wildcard: {:#}",
            none.body
        );
    }
}

/// Rules: ADR-0013 §3
#[tokio::test]
async fn test_rest_v2_a_body_is_refused_at_the_field_it_fails_at() {
    let f = fixture().await;
    seed(&f).await;

    // A field the body does not have, at any depth, in a tagged union, in a merge patch: refused,
    // never dropped.
    let misspelt = f
        .post("/api/v2/customers", json!({"name": "Typo", "alis": "typo"}))
        .await;
    unreadable_at(&misspelt, "/alis", "unknown");
    let nested = subscribe(&f, "unknown-nested", json!({"invoicing": {"net_term": 30}})).await;
    unreadable_at(&nested, "/invoicing/net_term", "unknown");
    let in_union = f
        .post(
            "/api/v2/plans",
            plan_of(
                "typo",
                &["monthly"],
                flat(json!({
                    "currency": "EUR",
                    "model": {"type": "flat", "amount": "10", "amout": "10"}
                })),
            ),
        )
        .await;
    unreadable_at(
        &in_union,
        "/draft/charges/0/prices/0/model/amout",
        "unknown",
    );
    let in_patch = f
        .patch("/api/v2/products/support", json!({"nmae": "Help"}))
        .await;
    unreadable_at(&in_patch, "/nmae", "unknown");

    // A wrong type and a missing field, at their pointer.
    let wrong_type = f.post("/api/v2/products", json!({"name": 5})).await;
    unreadable_at(&wrong_type, "/name", "invalid");
    let missing = f
        .post(
            "/api/v2/plans",
            json!({"code": "nameless", "name": "Nameless", "draft": {"charges": [{"preset": "flat_fee"}]}}),
        )
        .await;
    unreadable_at(&missing, "/draft/charges/0/product", "required");

    // Nothing was written by a refused body.
    let support = f.get("/api/v2/products/support").await;
    assert_eq!(support.body["name"], "Support", "{:#}", support.body);
}
