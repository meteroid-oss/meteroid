//! The public REST API v2 (ADR-0013): a plan written whole through its draft and published, read
//! back as its canonical composition; codes in paths; cursor pagination; problem errors; the
//! version header; idempotent POSTs.
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use meteroid_grpc::meteroid::api;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};

pub(crate) struct Fixture {
    pub(crate) setup: MeteroidSetup,
    pub(crate) client: reqwest::Client,
    pub(crate) api_key: String,
}

pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Value,
}

impl Answer {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

impl Fixture {
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.setup.config.rest_api_external_url)
    }

    pub(crate) async fn send(&self, request: reqwest::RequestBuilder) -> Answer {
        let response = request.send().await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap();
        Answer {
            status,
            headers,
            body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
        }
    }

    pub(crate) async fn get(&self, path: &str) -> Answer {
        self.send(self.client.get(self.url(path)).bearer_auth(&self.api_key))
            .await
    }

    pub(crate) async fn post(&self, path: &str, body: Value) -> Answer {
        self.send(
            self.client
                .post(self.url(path))
                .bearer_auth(&self.api_key)
                .json(&body),
        )
        .await
    }

    pub(crate) async fn post_once(&self, path: &str, key: &str, body: Value) -> Answer {
        self.send(
            self.client
                .post(self.url(path))
                .bearer_auth(&self.api_key)
                .header("Idempotency-Key", key)
                .json(&body),
        )
        .await
    }

    pub(crate) async fn put(&self, path: &str, body: Value) -> Answer {
        self.send(
            self.client
                .put(self.url(path))
                .bearer_auth(&self.api_key)
                .json(&body),
        )
        .await
    }

    pub(crate) async fn delete(&self, path: &str) -> Answer {
        self.send(
            self.client
                .delete(self.url(path))
                .bearer_auth(&self.api_key),
        )
        .await
    }

    /// A JSON merge patch.
    pub(crate) async fn patch(&self, path: &str, body: Value) -> Answer {
        self.send(
            self.client
                .patch(self.url(path))
                .bearer_auth(&self.api_key)
                .header("content-type", "application/merge-patch+json")
                .body(body.to_string()),
        )
        .await
    }
}

pub(crate) async fn fixture() -> Fixture {
    crate::helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
            name: "api-v2".to_owned(),
        })
        .await
        .unwrap()
        .into_inner()
        .api_key;
    Fixture {
        setup,
        client: http_client_tracing::client(),
        api_key,
    }
}

/// The draft of "studio": a platform fee per option, seats of a managed quantity in every option,
/// and metered usage.
fn pro_version(platform_monthly: &str) -> Value {
    json!({
        "currency": "EUR",
        "options": [
            {"alias": "monthly", "name": "Monthly"},
            {"alias": "annual", "name": "Annual"}
        ],
        "charges": [
            {
                "product": "platform",
                "preset": "flat_fee",
                "prices": [
                    {"option": "monthly", "currency": "EUR", "cadence": "monthly",
                     "model": {"type": "flat", "amount": platform_monthly}},
                    {"option": "annual", "currency": "EUR", "cadence": "annual",
                     "model": {"type": "flat", "amount": "490.00"}}
                ]
            },
            {
                "product": "seat",
                "preset": "seats",
                "quantity": {"type": "managed", "quantity": "users", "min": 1},
                "prices": [
                    {"currency": "EUR", "model": {"type": "per_unit", "unit_price": "10"}}
                ]
            },
            {
                "product": "api",
                "quantity": {"type": "metered", "metric": "bandwidth"},
                "prices": [
                    {"currency": "EUR", "model": {"type": "per_unit", "unit_price": "0.002"}}
                ]
            }
        ]
    })
}

pub(crate) async fn seed_catalog(f: &Fixture) {
    for (code, name) in [("platform", "Platform"), ("seat", "Seats"), ("api", "API")] {
        let created = f
            .post("/api/v2/products", json!({"code": code, "name": name}))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{:?}", created.body);
        assert_eq!(created.body["code"], code);
        assert!(
            created.body.get("fee_type").is_none(),
            "a product has no fee type"
        );
    }
    let quantity = f
        .post(
            "/api/v2/managed-quantities",
            json!({"code": "users", "name": "Users"}),
        )
        .await;
    assert_eq!(quantity.status, StatusCode::CREATED, "{:?}", quantity.body);
}

pub(crate) fn charge<'a>(version: &'a Value, product: &str) -> &'a Value {
    version["charges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|charge| charge["product"] == product)
        .unwrap_or_else(|| panic!("no charge of {product}: {version:#}"))
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_plan_authored_through_its_draft() {
    let f = fixture().await;
    seed_catalog(&f).await;

    // Created with its draft, once per Idempotency-Key.
    let create = json!({"code": "studio", "name": "Studio", "draft": pro_version("49")});
    let created = f
        .post_once("/api/v2/plans", "create-pro", create.clone())
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let plan_id = created.body["id"].as_str().unwrap().to_owned();
    assert_eq!(created.body["code"], "studio");
    assert_eq!(created.body["status"], "draft");
    assert_eq!(created.body["has_draft"], true);
    let draft = &created.body["version"];
    assert_eq!(draft["status"], "draft");
    assert_eq!(draft["number"], 1);
    assert_eq!(draft["currency"], "EUR");
    assert_eq!(draft["options"][0]["alias"], "monthly");
    assert_eq!(draft["options"][0]["default"], true);
    assert_eq!(draft["options"][1]["alias"], "annual");
    assert_eq!(
        draft["entitlements"],
        json!([]),
        "a draft written without rows"
    );

    let platform = charge(draft, "platform");
    assert_eq!(platform["preset"], "flat_fee");
    assert_eq!(platform["quantity"], json!({"type": "fixed", "value": "1"}));
    assert_eq!(
        platform["timing"],
        json!({"recurrence": "recurring", "billing": "advance"})
    );
    assert_eq!(platform["prices"][0]["option"], "monthly");
    assert_eq!(
        platform["prices"][0]["model"],
        json!({"type": "flat", "amount": "49"})
    );
    assert_eq!(platform["prices"][1]["option"], "annual");
    assert_eq!(platform["prices"][1]["cadence"], "annual");
    assert_eq!(
        platform["prices"][1]["model"]["amount"], "490",
        "decimal strings read normalized"
    );

    let seats = charge(draft, "seat");
    assert_eq!(seats["preset"], "seats");
    assert_eq!(
        seats["quantity"],
        json!({"type": "managed", "quantity": "users", "min": 1})
    );
    assert_eq!(seats["prices"][0]["option"], "*", "in every option");

    let usage = charge(draft, "api");
    assert_eq!(usage["preset"], "usage", "a hint of the composition");
    assert_eq!(
        usage["quantity"],
        json!({"type": "metered", "metric": "bandwidth"})
    );
    assert_eq!(usage["timing"]["billing"], "arrears");
    assert_eq!(
        usage["prices"][0]["model"],
        json!({"type": "per_unit", "unit_price": "0.002"})
    );

    // A retry replays the first answer; the key with another request is refused.
    let replayed = f
        .post_once("/api/v2/plans", "create-pro", create.clone())
        .await;
    assert_eq!(replayed.status, StatusCode::CREATED);
    assert_eq!(replayed.header("Idempotent-Replayed"), Some("true"));
    assert_eq!(replayed.body["id"], plan_id.as_str());
    let reused = f
        .post_once(
            "/api/v2/plans",
            "create-pro",
            json!({"code": "pro-2", "name": "Pro 2", "draft": pro_version("49")}),
        )
        .await;
    assert_eq!(reused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reused.body["code"], "idempotency_key_reused");

    // A claim its process left unanswered (a crash): refused while it may still run, taken over
    // once it is long past any request's run, and the request runs again.
    assert_eq!(left_unanswered(&f, "create-pro", 5).await, 1);
    let running = f
        .post_once("/api/v2/plans", "create-pro", create.clone())
        .await;
    assert_eq!(running.status, StatusCode::CONFLICT);
    assert_eq!(running.body["code"], "idempotency_request_in_progress");
    left_unanswered(&f, "create-pro", 11).await;
    let rerun = f
        .post_once("/api/v2/plans", "create-pro", create.clone())
        .await;
    assert_eq!(rerun.header("Idempotent-Replayed"), None, "it ran again");
    assert_ne!(
        rerun.status,
        StatusCode::CREATED,
        "the plan it made the first time holds its code"
    );

    // The draft written whole, twice: the same body reads the same.
    let first = f.put("/api/v2/plans/studio/draft", pro_version("59")).await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    let second = f.put("/api/v2/plans/studio/draft", pro_version("59")).await;
    assert_eq!(second.status, StatusCode::OK, "{:#}", second.body);
    assert_eq!(first.body["version"], second.body["version"]);
    // A draft read is written back as is: its number and status are the version's own.
    let again = f
        .put("/api/v2/plans/studio/draft", first.body["version"].clone())
        .await;
    assert_eq!(again.status, StatusCode::OK, "{:#}", again.body);
    assert_eq!(again.body["version"], first.body["version"]);
    assert_eq!(
        second.body["version"]["number"], 1,
        "the draft keeps its number"
    );
    assert_eq!(
        charge(&second.body["version"], "platform")["prices"][0]["model"]["amount"],
        "59"
    );

    // Published, then read back canonical, by code and by id.
    let published = f
        .post(&format!("/api/v2/plans/{plan_id}/draft/publish"), json!({}))
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:#}", published.body);
    assert_eq!(published.body["status"], "active");
    assert_eq!(published.body["active_version"], 1);
    assert_eq!(published.body["has_draft"], false);
    assert_eq!(published.body["version"]["status"], "active");

    let by_code = f.get("/api/v2/plans/studio").await;
    assert_eq!(by_code.status, StatusCode::OK, "{:#}", by_code.body);
    assert_eq!(by_code.body["id"], plan_id.as_str());
    let mut written = second.body["version"].clone();
    written["status"] = json!("active");
    assert_eq!(
        by_code.body["version"], written,
        "the published version reads as the draft written"
    );
    let by_id = f.get(&format!("/api/v2/plans/{plan_id}?version=1")).await;
    assert_eq!(by_id.body["version"], by_code.body["version"]);

    // A draft reading as the active version publishes nothing.
    let same = f.put("/api/v2/plans/studio/draft", pro_version("59")).await;
    assert_eq!(same.status, StatusCode::OK, "{:#}", same.body);
    assert_eq!(same.body["version"]["number"], 2);
    let noop = f
        .post("/api/v2/plans/studio/draft/publish", json!({}))
        .await;
    assert_eq!(noop.status, StatusCode::OK, "{:#}", noop.body);
    assert_eq!(noop.body["version"]["number"], 1);
    assert_eq!(noop.body["has_draft"], false);
    let versions = f.get("/api/v2/plans/studio/versions").await;
    assert_eq!(versions.status, StatusCode::OK, "{:#}", versions.body);
    let numbers: Vec<i64> = versions.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|version| version["number"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, vec![1]);

    // No draft left to publish.
    let none = f
        .post("/api/v2/plans/studio/draft/publish", json!({}))
        .await;
    assert_eq!(none.status, StatusCode::CONFLICT);
    assert_eq!(none.body["code"], "no_draft");

    // The plan is listed, without its versions.
    let plans = f.get("/api/v2/plans?status=active").await;
    assert_eq!(plans.status, StatusCode::OK, "{:#}", plans.body);
    let pro = plans.body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plan| plan["code"] == "studio")
        .expect("pro is listed");
    assert!(pro.get("version").is_none());
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_conventions() {
    let f = fixture().await;
    seed_catalog(&f).await;

    // The version header: the first version by default, a later date resolves to it, an earlier
    // one is refused, as a problem, the version still echoed.
    let plain = f.get("/api/v2/products").await;
    assert_eq!(plain.status, StatusCode::OK);
    assert_eq!(plain.header("Meteroid-Version"), Some("2026-12-01"));
    assert!(plain.header("Request-Id").is_some());
    let later = f
        .send(
            f.client
                .get(f.url("/api/v2/products"))
                .bearer_auth(&f.api_key)
                .header("Meteroid-Version", "2027-03-15"),
        )
        .await;
    assert_eq!(later.status, StatusCode::OK);
    assert_eq!(later.header("Meteroid-Version"), Some("2026-12-01"));
    let earlier = f
        .send(
            f.client
                .get(f.url("/api/v2/products"))
                .bearer_auth(&f.api_key)
                .header("Meteroid-Version", "2020-01-01"),
        )
        .await;
    assert_eq!(earlier.status, StatusCode::BAD_REQUEST);
    assert_eq!(earlier.body["code"], "unsupported_version");
    assert_eq!(earlier.header("Meteroid-Version"), Some("2026-12-01"));

    // Problems: specific codes, the request id in the body and the header.
    let missing = f
        .send(
            f.client
                .get(f.url("/api/v2/plans/nope"))
                .bearer_auth(&f.api_key)
                .header("X-Request-Id", "req-from-caller"),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(
        missing.header("content-type"),
        Some("application/problem+json")
    );
    assert_eq!(missing.body["code"], "not_found");
    assert_eq!(missing.body["status"], 404);
    assert_eq!(missing.body["request_id"], "req-from-caller");
    assert_eq!(missing.header("Request-Id"), Some("req-from-caller"));

    let anonymous = f.send(f.client.get(f.url("/api/v2/products"))).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.body["code"], "unauthorized");
    assert_eq!(anonymous.header("WWW-Authenticate"), Some("Bearer"));
    assert!(anonymous.body["request_id"].is_string());

    let unknown = f.get("/api/v2/nothing-here").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body["code"], "not_found");

    let unreadable = f
        .send(
            f.client
                .post(f.url("/api/v2/products"))
                .bearer_auth(&f.api_key)
                .header("content-type", "application/json")
                .body("{"),
        )
        .await;
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);
    assert_eq!(unreadable.body["code"], "invalid_request");

    // A PATCH reads as JSON whichever of its two content types it is sent with.
    for (content_type, name) in [
        ("application/json", "Platform fee"),
        ("application/merge-patch+json", "Platform"),
    ] {
        let patched = f
            .send(
                f.client
                    .patch(f.url("/api/v2/products/platform"))
                    .bearer_auth(&f.api_key)
                    .header("content-type", content_type)
                    .body(json!({"name": name}).to_string()),
            )
            .await;
        assert_eq!(
            patched.status,
            StatusCode::OK,
            "{content_type}: {:#}",
            patched.body
        );
        assert_eq!(patched.body["name"], name);
    }

    // What the model cannot express is refused at its JSON pointer.
    let flat_seats = f
        .post(
            "/api/v2/plans",
            json!({
                "name": "Flat seats",
                "draft": {"charges": [{
                    "product": "seat",
                    "quantity": {"type": "managed", "quantity": "users"},
                    "prices": [{"currency": "EUR", "model": {"type": "flat", "amount": "10"}}]
                }]}
            }),
        )
        .await;
    assert_eq!(flat_seats.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(flat_seats.body["code"], "not_expressible");
    assert_eq!(
        flat_seats.body["errors"][0]["path"],
        "/draft/charges/0/prices/0/model"
    );
    let unknown_product = f
        .post(
            "/api/v2/plans",
            json!({
                "name": "Ghost",
                "draft": {"charges": [{"product": "ghost", "preset": "flat_fee", "prices": []}]}
            }),
        )
        .await;
    assert_eq!(unknown_product.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        unknown_product.body["errors"][0]["path"],
        "/draft/charges/0/product"
    );

    // A code is unique per tenant.
    let taken = f
        .post(
            "/api/v2/products",
            json!({"code": "platform", "name": "Again"}),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    assert_eq!(taken.body["code"], "code_taken");

    // Codes in paths, beside ids.
    let by_code = f.get("/api/v2/products/platform").await;
    assert_eq!(by_code.status, StatusCode::OK);
    let id = by_code.body["id"].as_str().unwrap().to_owned();
    let by_id = f.get(&format!("/api/v2/products/{id}")).await;
    assert_eq!(by_id.body["code"], "platform");
    let quantity = f.get("/api/v2/managed-quantities/users").await;
    assert_eq!(quantity.body["name"], "Users");
    let metric = f.get("/api/v2/metrics/bandwidth").await;
    assert_eq!(metric.status, StatusCode::OK, "{:#}", metric.body);
    assert_eq!(metric.body["aggregation"], "sum");

    // Cursor pagination: pages follow each other, each item once.
    let mut seen: Vec<String> = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let path = match &after {
            Some(cursor) => format!("/api/v2/products?limit=2&after={cursor}"),
            None => "/api/v2/products?limit=2".to_owned(),
        };
        let page = f.get(&path).await;
        assert_eq!(page.status, StatusCode::OK, "{:#}", page.body);
        let data = page.body["data"].as_array().unwrap();
        assert!(data.len() <= 2);
        seen.extend(data.iter().map(|p| p["code"].as_str().unwrap().to_owned()));
        if page.body["has_more"] == false {
            assert!(page.body["next_cursor"].is_null());
            break;
        }
        after = Some(page.body["next_cursor"].as_str().unwrap().to_owned());
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "no item twice: {seen:?}");
    for code in ["platform", "seat", "api"] {
        assert!(
            seen.iter().any(|seen| seen == code),
            "{code} listed: {seen:?}"
        );
    }
    let refused = f.get("/api/v2/products?limit=0").await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.body["errors"][0]["path"], "limit");

    // The v2 document is served.
    let spec = f
        .send(f.client.get(f.url("/api-docs/v2/openapi.json")))
        .await;
    assert_eq!(spec.status, StatusCode::OK);
    assert!(spec.body["paths"]["/api/v2/plans/{id_or_code}/draft"].is_object());
}

/// The data of a page, as a list.
pub(crate) fn listed(answer: &Answer) -> &Vec<Value> {
    answer.body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("not a page: {:#}", answer.body))
}

/// What each row of an owner's list grants, without its identity (ids, owner, dates).
pub(crate) fn granted(rows: &Value) -> Vec<Value> {
    rows.as_array()
        .unwrap_or_else(|| panic!("not a list of rows: {rows:#}"))
        .iter()
        .map(|row| {
            let mut grant = json!({
                "feature": row["feature"],
                "value": row["value"],
                "effect": row["effect"],
                "level": row["level"],
            });
            for optional in ["resource_type", "condition", "valid_from", "valid_until"] {
                if let Some(value) = row.get(optional) {
                    grant[optional] = value.clone();
                }
            }
            grant
        })
        .collect()
}

/// The row's id, a lineage's.
pub(crate) fn row_id(row: &Value) -> String {
    row["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a row has an id: {row:#}"))
        .to_owned()
}

/// A 422 whose first refusal is `code` at `path`.
pub(crate) fn refused(answer: &Answer, path: &str, code: &str) {
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{path}: {:#}",
        answer.body
    );
    assert_eq!(answer.body["errors"][0]["path"], path, "{:#}", answer.body);
    assert_eq!(answer.body["errors"][0]["code"], code, "{:#}", answer.body);
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_metrics_and_rate_cards() {
    let f = fixture().await;

    // A metric: created, read by code, updated, archived; its code stays taken once archived.
    let created = f
        .post(
            "/api/v2/metrics",
            json!({
                "code": "llm_tokens",
                "name": "Tokens",
                "aggregation": "sum",
                "aggregation_key": "tokens",
                "unit_singular": "token",
                "unit_plural": "tokens",
                "unit_conversion": {"factor": 1000000, "rounding": "up"},
                "filters": [{"property": "model", "op": "in", "values": ["fast", "smart"]}],
                "group_by": "region"
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let metric_id = created.body["id"].as_str().unwrap().to_owned();
    assert_eq!(created.body["code"], "llm_tokens");
    assert_eq!(
        created.body["event_code"], "llm_tokens",
        "the code when left out"
    );
    assert_eq!(created.body["aggregation"], "sum");
    assert_eq!(
        created.body["unit_conversion"],
        json!({"factor": 1000000, "rounding": "up"})
    );
    assert_eq!(created.body["filters"][0]["op"], "in");
    assert_eq!(created.body["group_by"], "region");
    assert!(created.body["family"]["code"].is_string());
    assert!(
        created.body["family"]["id"]
            .as_str()
            .unwrap()
            .starts_with("pf_")
    );

    let taken = f
        .post(
            "/api/v2/metrics",
            json!({"code": "llm_tokens", "name": "Again", "aggregation": "count"}),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{:#}", taken.body);
    assert_eq!(taken.body["code"], "code_taken");
    assert_eq!(taken.body["errors"][0]["path"], "/code");
    let uncoded = f
        .post(
            "/api/v2/metrics",
            json!({"name": "Uncoded", "aggregation": "count"}),
        )
        .await;
    assert_eq!(uncoded.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(uncoded.body["errors"][0]["path"], "/code");

    // Other metrics of its event code, aggregating its events their own way: each its own code,
    // the event code numbered when left out.
    let peak = f
        .post(
            "/api/v2/metrics",
            json!({"event_code": "llm_tokens", "name": "Peak tokens", "aggregation": "max",
                   "aggregation_key": "tokens"}),
        )
        .await;
    assert_eq!(peak.status, StatusCode::CREATED, "{:#}", peak.body);
    assert_eq!(peak.body["code"], "llm_tokens-2");
    assert_eq!(peak.body["event_code"], "llm_tokens");
    let calls = f
        .post(
            "/api/v2/metrics",
            json!({"code": "llm_calls", "event_code": "llm_tokens", "name": "LLM calls",
                   "aggregation": "count"}),
        )
        .await;
    assert_eq!(calls.status, StatusCode::CREATED, "{:#}", calls.body);
    assert_eq!(calls.body["code"], "llm_calls");
    assert_eq!(calls.body["event_code"], "llm_tokens");
    for (code, created) in [("llm_tokens-2", &peak), ("llm_calls", &calls)] {
        let read = f.get(&format!("/api/v2/metrics/{code}")).await;
        assert_eq!(read.status, StatusCode::OK, "{code}: {:#}", read.body);
        assert_eq!(read.body["id"], created.body["id"], "{code}");
    }
    let keyless = f
        .post(
            "/api/v2/metrics",
            json!({"code": "keyless", "name": "Keyless", "aggregation": "sum"}),
        )
        .await;
    assert_eq!(keyless.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(keyless.body["errors"][0]["path"], "/aggregation_key");

    let by_code = f.get("/api/v2/metrics/llm_tokens").await;
    assert_eq!(by_code.status, StatusCode::OK, "{:#}", by_code.body);
    assert_eq!(by_code.body["id"], metric_id.as_str());

    let updated = f
        .patch(
            "/api/v2/metrics/llm_tokens",
            json!({"name": "LLM tokens", "unit_conversion": null, "filters": []}),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{:#}", updated.body);
    assert_eq!(updated.body["name"], "LLM tokens");
    assert!(updated.body.get("unit_conversion").is_none());
    assert_eq!(updated.body["filters"], json!([]));
    assert_eq!(updated.body["unit_plural"], "tokens", "left out, kept");
    let blank = f
        .patch("/api/v2/metrics/llm_tokens", json!({"name": " "}))
        .await;
    assert_eq!(blank.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A rate card on it: created, rated, renamed, its changes read newest first.
    let card = f
        .post(
            "/api/v2/rate-cards",
            json!({
                "code": "llm-rates",
                "name": "LLM rates",
                "metric": "llm_tokens",
                "dimensions": ["model"],
                "unit_size": "1000000"
            }),
        )
        .await;
    assert_eq!(card.status, StatusCode::CREATED, "{:#}", card.body);
    assert_eq!(card.body["metric"]["code"], "llm_tokens");
    assert!(
        card.body["metric"]["id"]
            .as_str()
            .unwrap()
            .starts_with("bm_")
    );
    assert_eq!(card.body["denomination"], "money");
    assert_eq!(card.body["rates"], json!([]));
    let twice = f
        .post(
            "/api/v2/rate-cards",
            json!({"code": "llm-rates", "name": "Again", "metric": "llm_tokens",
                   "dimensions": ["model"]}),
        )
        .await;
    assert_eq!(twice.status, StatusCode::CONFLICT, "{:#}", twice.body);
    assert_eq!(twice.body["code"], "code_taken");

    let rated = f
        .patch(
            "/api/v2/rate-cards/llm-rates/rates",
            json!({"rates": [{"key": ["fast"], "currency": "EUR", "rate": "2"}]}),
        )
        .await;
    assert_eq!(rated.status, StatusCode::OK, "{:#}", rated.body);
    assert_eq!(rated.body["inserted"], 1);

    let renamed = f
        .patch(
            "/api/v2/rate-cards/llm-rates",
            json!({"name": "LLM prices", "on_unmatched": "ignore"}),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK, "{:#}", renamed.body);
    assert_eq!(renamed.body["name"], "LLM prices");
    assert_eq!(renamed.body["on_unmatched"], "ignore");
    assert_eq!(renamed.body["rates"][0]["rate"], "2");

    let changes = f.get("/api/v2/rate-cards/llm-rates/changes").await;
    assert_eq!(changes.status, StatusCode::OK, "{:#}", changes.body);
    let kinds: Vec<&str> = listed(&changes)
        .iter()
        .map(|change| change["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["updated", "rates_set", "created"]);
    let batch = &listed(&changes)[1];
    assert_eq!(batch["rates"][0]["key"], json!(["fast"]));
    assert_eq!(batch["rates"][0]["to"], "2");
    assert!(batch["rates"][0].get("from").is_none(), "a first rate");
    assert_eq!(batch["actor"]["type"], "api_token");
    let first = f.get("/api/v2/rate-cards/llm-rates/changes?limit=1").await;
    assert_eq!(listed(&first).len(), 1);
    assert_eq!(first.body["has_more"], true);
    let next = f
        .get(&format!(
            "/api/v2/rate-cards/llm-rates/changes?limit=1&after={}",
            first.body["next_cursor"].as_str().unwrap()
        ))
        .await;
    assert_eq!(listed(&next)[0]["kind"], "rates_set");

    // Archived, the metric keeps its code: still read by it, taken for a new metric; unarchived
    // as it was.
    let archived = f
        .post("/api/v2/metrics/llm_tokens/archive", json!({}))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["archived"], true);
    let live = f.get("/api/v2/metrics").await;
    assert!(!listed(&live).iter().any(|m| m["id"] == metric_id.as_str()));
    let all = f.get("/api/v2/metrics?include_archived=true").await;
    assert!(listed(&all).iter().any(|m| m["id"] == metric_id.as_str()));
    let still = f.get("/api/v2/metrics/llm_tokens").await;
    assert_eq!(still.status, StatusCode::OK, "{:#}", still.body);
    assert_eq!(still.body["archived"], true);
    let again = f
        .post(
            "/api/v2/metrics",
            json!({"code": "llm_tokens", "name": "Tokens v2", "aggregation": "count"}),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{:#}", again.body);
    assert_eq!(again.body["code"], "code_taken");
    let back = f
        .post(&format!("/api/v2/metrics/{metric_id}/unarchive"), json!({}))
        .await;
    assert_eq!(back.status, StatusCode::OK, "{:#}", back.body);
    assert_eq!(back.body["archived"], false);
    assert_eq!(back.body["code"], "llm_tokens");
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_entitlements_and_archives() {
    let f = fixture().await;
    seed_catalog(&f).await;
    for feature in [
        json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"}, "product": "platform"}),
        json!({"code": "api-calls", "name": "API calls",
               "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"},
               "product": "api"}),
    ] {
        let created = f.post("/api/v2/features", feature).await;
        assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    }
    let sso = json!({"feature": "sso", "value": {"type": "on"}});
    let calls = json!({"feature": "api-calls", "value": {"type": "limit", "limit": "1000"}});
    let grant = |feature: &str, value: Value, effect: &str| json!({"feature": feature, "value": value, "effect": effect, "level": "customer"});

    // A plan's draft carries its rows: written with it, read back canonical, owned by the version.
    let mut draft = pro_version("49");
    draft["entitlements"] = json!([sso.clone()]);
    let created = f
        .post(
            "/api/v2/plans",
            json!({"code": "granted", "name": "Granted", "draft": draft}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let rows = &created.body["version"]["entitlements"];
    assert_eq!(
        granted(rows),
        vec![grant("sso", json!({"type": "on"}), "set")]
    );
    assert_eq!(rows[0]["owner"]["type"], "plan_version");
    assert!(row_id(&rows[0]).starts_with("ent_"), "{rows:#}");

    // Left out, they are kept; sent, they replace the draft's.
    let kept = f
        .put("/api/v2/plans/granted/draft", pro_version("49"))
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{:#}", kept.body);
    assert_eq!(
        granted(&kept.body["version"]["entitlements"]),
        granted(rows)
    );
    let mut both = pro_version("49");
    both["entitlements"] = json!([calls.clone(), sso.clone()]);
    let replaced = f.put("/api/v2/plans/granted/draft", both.clone()).await;
    assert_eq!(replaced.status, StatusCode::OK, "{:#}", replaced.body);
    assert_eq!(
        granted(&replaced.body["version"]["entitlements"]),
        vec![
            grant(
                "api-calls",
                json!({"type": "limit", "limit": "1000"}),
                "set"
            ),
            grant("sso", json!({"type": "on"}), "set"),
        ],
        "sorted by feature"
    );
    let mut mistyped = pro_version("49");
    mistyped["entitlements"] =
        json!([{"feature": "sso", "value": {"type": "limit", "limit": "1"}}]);
    let refused = f.put("/api/v2/plans/granted/draft", mistyped).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    assert_eq!(refused.body["errors"][0]["path"], "/entitlements/0/value");

    let published = f
        .post("/api/v2/plans/granted/draft/publish", json!({}))
        .await;
    assert_eq!(published.status, StatusCode::OK, "{:#}", published.body);
    assert_eq!(
        published.body["version"]["entitlements"], replaced.body["version"]["entitlements"],
        "the draft's rows are the version's"
    );

    // A draft differing only in what it grants is a new version.
    let mut fewer = pro_version("49");
    fewer["entitlements"] = json!([sso.clone()]);
    let changed = f.put("/api/v2/plans/granted/draft", fewer).await;
    assert_eq!(changed.status, StatusCode::OK, "{:#}", changed.body);
    let second = f
        .post("/api/v2/plans/granted/draft/publish", json!({}))
        .await;
    assert_eq!(second.status, StatusCode::OK, "{:#}", second.body);
    assert_eq!(second.body["version"]["number"], 2);
    assert_eq!(
        listed(&f.get("/api/v2/plans/granted/versions").await).len(),
        2
    );

    // An add-on grants what it is created with, and what a patch replaces it with; bought any
    // number of times, its rows add up.
    let add_on = f
        .post(
            "/api/v2/add-ons",
            json!({
                "code": "more-calls",
                "name": "More calls",
                "charge": {"product": "api", "preset": "flat_fee", "prices": [
                    {"currency": "EUR", "cadence": "monthly", "model": {"type": "flat", "amount": "20"}}
                ]},
                "entitlements": [calls.clone()]
            }),
        )
        .await;
    assert_eq!(add_on.status, StatusCode::CREATED, "{:#}", add_on.body);
    assert_eq!(
        granted(&add_on.body["entitlements"]),
        vec![grant(
            "api-calls",
            json!({"type": "limit", "limit": "1000"}),
            "add"
        )]
    );
    let emptied = f
        .patch("/api/v2/add-ons/more-calls", json!({"entitlements": []}))
        .await;
    assert_eq!(emptied.status, StatusCode::OK, "{:#}", emptied.body);
    assert_eq!(emptied.body["entitlements"], json!([]));

    // A product holds the defaults of its own features.
    let product = f
        .patch(
            "/api/v2/products/platform",
            json!({"entitlements": [sso.clone()]}),
        )
        .await;
    assert_eq!(product.status, StatusCode::OK, "{:#}", product.body);
    let defaults = granted(&product.body["entitlements"]);
    assert_eq!(defaults, vec![grant("sso", json!({"type": "on"}), "set")]);
    // Its fields and its rows are written together: a refused row writes neither.
    let foreign = f
        .patch(
            "/api/v2/products/platform",
            json!({"name": "Platform v2", "description": "Renamed",
                   "entitlements": [calls.clone()]}),
        )
        .await;
    assert_eq!(
        foreign.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        foreign.body
    );
    assert_eq!(foreign.body["errors"][0]["path"], "/entitlements/0/feature");
    let unchanged = f.get("/api/v2/products/platform").await;
    assert_eq!(unchanged.body["name"], "Platform", "nothing half written");
    assert!(
        unchanged.body.get("description").is_none(),
        "{:#}",
        unchanged.body
    );
    assert_eq!(granted(&unchanged.body["entitlements"]), defaults);
    let feature = f.get("/api/v2/features/sso").await;
    assert_eq!(granted(&feature.body["defaults"]), defaults);
    assert_eq!(feature.body["defaults"][0]["owner"]["type"], "feature");
    assert_eq!(
        feature.body["defaults"][0]["owner"]["id"],
        feature.body["id"]
    );

    // Archived and back: plans, products, add-ons.
    let spare = f
        .post(
            "/api/v2/products",
            json!({"code": "spare", "name": "Spare"}),
        )
        .await;
    assert_eq!(spare.status, StatusCode::CREATED, "{:#}", spare.body);
    for (path, archived_field, archived_value, live_value) in [
        (
            "/api/v2/plans/granted",
            "status",
            json!("archived"),
            json!("active"),
        ),
        (
            "/api/v2/products/spare",
            "archived",
            json!(true),
            json!(false),
        ),
        (
            "/api/v2/add-ons/more-calls",
            "archived",
            json!(true),
            json!(false),
        ),
    ] {
        let archived = f.post(&format!("{path}/archive"), json!({})).await;
        assert_eq!(
            archived.status,
            StatusCode::OK,
            "{path}: {:#}",
            archived.body
        );
        assert_eq!(archived.body[archived_field], archived_value, "{path}");
        let again = f.post(&format!("{path}/archive"), json!({})).await;
        assert_eq!(
            again.status,
            StatusCode::OK,
            "{path}: archiving twice changes nothing"
        );
        let back = f.post(&format!("{path}/unarchive"), json!({})).await;
        assert_eq!(back.status, StatusCode::OK, "{path}: {:#}", back.body);
        assert_eq!(back.body[archived_field], live_value, "{path}");
    }
    let archived = f.post("/api/v2/products/spare/archive", json!({})).await;
    assert_eq!(archived.status, StatusCode::OK);
    let products = f.get("/api/v2/products").await;
    assert!(!listed(&products).iter().any(|p| p["code"] == "spare"));
    let with_archived = f.get("/api/v2/products?include_archived=true").await;
    assert!(listed(&with_archived).iter().any(|p| p["code"] == "spare"));

    // A plan's PATCH writes its fields and its listing together; a refused listing writes neither.
    let refused = f
        .patch(
            "/api/v2/plans/granted",
            json!({"name": "Renamed", "listing": {"contact_url": "not a url"}}),
        )
        .await;
    assert!(refused.status.is_client_error(), "{:#}", refused.body);
    let unchanged = f.get("/api/v2/plans/granted").await;
    assert_eq!(unchanged.body["name"], "Granted", "nothing half written");
    let patched = f
        .patch(
            "/api/v2/plans/granted",
            json!({"name": "Renamed", "listing": {"tagline": "For teams", "popular": true}}),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["name"], "Renamed");
    assert_eq!(patched.body["listing"]["tagline"], "For teams");
}

/// A key of a second tenant of the same organization.
async fn second_tenant_key(f: &Fixture) -> String {
    let auth = meteroid_it::svc_auth::login(f.setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let tenant = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "other".to_owned(),
            environment: api::tenants::v1::TenantEnvironmentEnum::Development as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();
    let other = AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        tenant.slug.as_str(),
    );
    other
        .api_tokens
        .clone()
        .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
            name: "other".to_owned(),
        })
        .await
        .unwrap()
        .into_inner()
        .api_key
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_references_stay_in_their_tenant() {
    let f = fixture().await;
    seed_catalog(&f).await;
    let other_key = second_tenant_key(&f).await;
    let as_other = |path: &str, body: Value| {
        f.client
            .post(f.url(path))
            .bearer_auth(&other_key)
            .json(&body)
    };

    // The other tenant's catalog.
    let product = f
        .send(as_other(
            "/api/v2/products",
            json!({"code": "theirs", "name": "Theirs"}),
        ))
        .await;
    assert_eq!(product.status, StatusCode::CREATED, "{:#}", product.body);
    let product_id = product.body["id"].as_str().unwrap().to_owned();
    let add_on = f
        .send(as_other(
            "/api/v2/add-ons",
            json!({"code": "their-add-on", "name": "Theirs", "charge": {
            "product": "theirs", "preset": "flat_fee", "prices": [
                {"currency": "EUR", "cadence": "monthly", "model": {"type": "flat", "amount": "5"}}
            ]}}),
        ))
        .await;
    assert_eq!(add_on.status, StatusCode::CREATED, "{:#}", add_on.body);
    let add_on_id = add_on.body["id"].as_str().unwrap().to_owned();
    let plan = f
        .send(as_other(
            "/api/v2/plans",
            json!({"code": "their-plan", "name": "Theirs", "draft": {"charges": [{
            "product": "theirs", "preset": "flat_fee", "prices": [
                {"currency": "EUR", "cadence": "monthly", "model": {"type": "flat", "amount": "5"}}
            ]}]}}),
        ))
        .await;
    assert_eq!(plan.status, StatusCode::CREATED, "{:#}", plan.body);
    let plan_id = plan.body["id"].as_str().unwrap().to_owned();

    // Their ids read as not found here, by path and in a body, and nothing is written.
    for path in [
        format!("/api/v2/products/{product_id}"),
        format!("/api/v2/add-ons/{add_on_id}"),
        format!("/api/v2/plans/{plan_id}"),
    ] {
        let read = f.get(&path).await;
        assert_eq!(
            read.status,
            StatusCode::NOT_FOUND,
            "{path}: {:#}",
            read.body
        );
    }
    let mut with_their_add_on = pro_version("49");
    with_their_add_on["add_ons"] = json!([{"add_on": add_on_id}]);
    let refused = f
        .post(
            "/api/v2/plans",
            json!({"code": "borrower", "name": "Borrower", "draft": with_their_add_on}),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    assert_eq!(refused.body["errors"][0]["path"], "/draft/add_ons/0/add_on");
    let mut with_their_trial = pro_version("49");
    with_their_trial["trial"] = json!({"days": 7, "plan": plan_id});
    let refused = f
        .post(
            "/api/v2/plans",
            json!({"code": "borrower", "name": "Borrower", "draft": with_their_trial}),
        )
        .await;
    assert!(
        refused.status == StatusCode::NOT_FOUND
            || refused.status == StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    let with_their_product = json!({"code": "borrower", "name": "Borrower", "draft": {"charges": [{
    "product": product_id, "preset": "flat_fee", "prices": [
        {"currency": "EUR", "cadence": "monthly", "model": {"type": "flat", "amount": "5"}}
    ]}]}});
    let refused = f.post("/api/v2/plans", with_their_product).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    assert_eq!(
        refused.body["errors"][0]["path"],
        "/draft/charges/0/product"
    );
    let none = f.get("/api/v2/plans/borrower").await;
    assert_eq!(none.status, StatusCode::NOT_FOUND, "nothing written");
    let coupon = f
        .post(
            "/api/v2/coupons",
            json!({"code": "BORROWED", "discount": {"type": "percentage", "percentage": "10"},
                   "applies_to": {"plans": [plan_id]}}),
        )
        .await;
    assert_eq!(
        coupon.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        coupon.body
    );
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_features() {
    let f = fixture().await;
    seed_catalog(&f).await;

    let created = f
        .post(
            "/api/v2/features",
            json!({
                "code": "api-calls",
                "name": "API calls",
                "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth",
                         "policy": {"resets": {"type": "billing_cycle"}, "at_limit": "deny"}},
                "product": "api",
                "unit": "calls",
                "defaults": [{"feature": "api-calls", "value": {"type": "limit", "limit": "1000.0"}}]
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let id = created.body["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("feat_"));
    assert_eq!(
        created.body["type"],
        json!({"type": "limit", "counted_by": "events", "metric": "bandwidth",
               "policy": {"resets": {"type": "billing_cycle"}, "at_limit": "deny",
                          "grace_pct": 0, "thresholds": []}})
    );
    assert_eq!(created.body["product"]["code"], "api");
    assert_eq!(created.body["unit"], "calls");
    assert_eq!(created.body["status"], "active");
    assert!(created.body.get("applies_per").is_none(), "the customer's");
    let default = &created.body["defaults"][0];
    assert_eq!(
        default["value"],
        json!({"type": "limit", "limit": "1000"}),
        "decimal strings read normalized"
    );
    assert_eq!(default["effect"], "set");
    assert_eq!(
        default["level"], "customer",
        "the feature's own level, resolved"
    );
    assert_eq!(default["owner"], json!({"type": "feature", "id": id}));
    let default_id = row_id(default);

    let taken = f
        .post(
            "/api/v2/features",
            json!({"code": "api-calls", "name": "Again", "type": {"type": "boolean"}}),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT, "{:#}", taken.body);
    assert_eq!(taken.body["code"], "code_taken");
    let mistyped = f
        .post(
            "/api/v2/features",
            json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"},
                   "defaults": [{"feature": "sso", "value": {"type": "limit", "limit": "1"}}]}),
        )
        .await;
    refused(&mistyped, "/defaults/0/value", "invalid");
    assert_eq!(
        f.get("/api/v2/features/sso").await.status,
        StatusCode::NOT_FOUND,
        "nothing written"
    );

    let by_code = f.get("/api/v2/features/api-calls").await;
    assert_eq!(by_code.status, StatusCode::OK, "{:#}", by_code.body);
    assert_eq!(by_code.body["id"], id.as_str());

    // A patch merges the type's policy and replaces the defaults; a changed default keeps its id.
    let updated = f
        .patch(
            "/api/v2/features/api-calls",
            json!({"name": "API requests",
                   "type": {"policy": {"resets": {"type": "billing_cycle"}, "at_limit": "deny",
                                       "grace_pct": 10, "thresholds": [80, 50]}},
                   "defaults": [{"feature": "api-calls", "value": {"type": "limit", "limit": "5000"}}]}),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{:#}", updated.body);
    assert_eq!(updated.body["name"], "API requests");
    assert_eq!(updated.body["type"]["policy"]["grace_pct"], 10);
    assert_eq!(
        updated.body["type"]["policy"]["thresholds"],
        json!([50, 80])
    );
    assert_eq!(
        updated.body["type"]["metric"], "bandwidth",
        "left out, kept"
    );
    assert_eq!(updated.body["defaults"][0]["value"]["limit"], "5000");
    assert_eq!(row_id(&updated.body["defaults"][0]), default_id);
    let cleared = f
        .patch("/api/v2/features/api-calls", json!({"defaults": []}))
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{:#}", cleared.body);
    assert_eq!(cleared.body["defaults"], json!([]));

    let of_api = f.get("/api/v2/features?product=api").await;
    assert_eq!(of_api.status, StatusCode::OK, "{:#}", of_api.body);
    assert!(listed(&of_api).iter().any(|x| x["code"] == "api-calls"));

    // Archived: hidden from the list, its code free; the code still names it while no live
    // feature holds it.
    let archived = f
        .post("/api/v2/features/api-calls/archive", json!({}))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["status"], "archived");
    let by_code = f.get("/api/v2/features/api-calls").await;
    assert_eq!(by_code.status, StatusCode::OK, "{:#}", by_code.body);
    assert_eq!(by_code.body["id"], id.as_str());
    assert_eq!(by_code.body["status"], "archived");
    let by_id = f.get(&format!("/api/v2/features/{id}")).await;
    assert_eq!(by_id.body["status"], "archived");
    let all = f.get("/api/v2/features").await;
    assert!(
        !listed(&all).iter().any(|x| x["code"] == "api-calls"),
        "archived features are left out"
    );
    let asked = f.get("/api/v2/features?status=archived").await;
    assert!(listed(&asked).iter().any(|x| x["id"] == id.as_str()));

    let restored = f
        .post("/api/v2/features/api-calls/unarchive", json!({}))
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{:#}", restored.body);
    assert_eq!(restored.body["status"], "active");
    assert_eq!(restored.body["id"], id.as_str());
}

/// Rules: ADR-0013
#[tokio::test]
async fn test_rest_v2_coupons_and_credit_units() {
    let f = fixture().await;
    seed_catalog(&f).await;
    let plan = f
        .post(
            "/api/v2/plans",
            json!({"code": "solo", "name": "Solo", "draft": {"currency": "EUR", "charges": [{
                "product": "platform", "preset": "flat_fee",
                "prices": [{"currency": "EUR", "cadence": "monthly",
                            "model": {"type": "flat", "amount": "10"}}]
            }]}}),
        )
        .await;
    assert_eq!(plan.status, StatusCode::CREATED, "{:#}", plan.body);

    // Coupons: created, read by code, updated, disabled, archived.
    let created = f
        .post(
            "/api/v2/coupons",
            json!({
                "code": "SPRING20",
                "description": "Spring",
                "discount": {"type": "percentage", "percentage": "20"},
                "duration": {"type": "months", "months": 3},
                "applies_to": {"plans": ["solo"]}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let coupon_id = created.body["id"].as_str().unwrap().to_owned();
    assert!(coupon_id.starts_with("cou_"));
    assert_eq!(
        created.body["discount"],
        json!({"type": "percentage", "percentage": "20"})
    );
    assert_eq!(
        created.body["duration"],
        json!({"type": "months", "months": 3})
    );
    assert_eq!(created.body["applies_to"]["plans"], json!(["solo"]));

    let by_code = f.get("/api/v2/coupons/SPRING20").await;
    assert_eq!(by_code.status, StatusCode::OK, "{:#}", by_code.body);
    assert_eq!(by_code.body["id"], coupon_id.as_str());
    let any_case = f.get("/api/v2/coupons/spring20").await;
    assert_eq!(
        any_case.body["id"],
        coupon_id.as_str(),
        "codes match ignoring case"
    );

    let taken = f
        .post(
            "/api/v2/coupons",
            json!({"code": "SPRING20", "discount": {"type": "percentage", "percentage": "5"}}),
        )
        .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);
    assert_eq!(taken.body["code"], "code_taken");

    let updated = f
        .patch(
            "/api/v2/coupons/SPRING20",
            json!({
                "description": null,
                "discount": {"type": "fixed", "currency": "EUR", "amount": "5",
                             "amounts": {"USD": "6"}},
                "duration": {"type": "forever"}
            }),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{:#}", updated.body);
    assert!(updated.body.get("description").is_none());
    assert_eq!(updated.body["discount"]["amounts"]["USD"], "6");
    assert_eq!(updated.body["duration"]["type"], "forever");
    let invoices = f
        .patch(
            "/api/v2/coupons/SPRING20",
            json!({"duration": {"type": "invoices", "invoices": 2}}),
        )
        .await;
    assert_eq!(invoices.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invoices.body["errors"][0]["path"], "/duration/type");

    let disabled = f.post("/api/v2/coupons/SPRING20/disable", json!({})).await;
    assert_eq!(disabled.status, StatusCode::OK, "{:#}", disabled.body);
    assert_eq!(disabled.body["disabled"], true);
    let enabled = f.post("/api/v2/coupons/SPRING20/enable", json!({})).await;
    assert_eq!(enabled.body["disabled"], false);

    let archived = f.post("/api/v2/coupons/SPRING20/archive", json!({})).await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["archived"], true);
    let still = f.get("/api/v2/coupons/SPRING20").await;
    assert_eq!(
        still.body["id"],
        coupon_id.as_str(),
        "an archived coupon reads by its code"
    );
    let live = f.get("/api/v2/coupons").await;
    assert!(
        !listed(&live).iter().any(|c| c["id"] == coupon_id.as_str()),
        "archived coupons are left out by default"
    );
    let archived_list = f.get("/api/v2/coupons?status=archived").await;
    assert!(
        listed(&archived_list)
            .iter()
            .any(|c| c["id"] == coupon_id.as_str())
    );

    // The code is free again; the archived coupon cannot take it back.
    let reused = f
        .post(
            "/api/v2/coupons",
            json!({"code": "SPRING20", "discount": {"type": "percentage", "percentage": "10"}}),
        )
        .await;
    assert_eq!(reused.status, StatusCode::CREATED, "{:#}", reused.body);
    let back = f
        .post(&format!("/api/v2/coupons/{coupon_id}/unarchive"), json!({}))
        .await;
    assert_eq!(back.status, StatusCode::CONFLICT);
    assert_eq!(back.body["code"], "code_taken");

    // Credit units: created with a product and prices, read by code, updated, archived.
    let unit = f
        .post(
            "/api/v2/credit-units",
            json!({"code": "TOKENS", "name": "Token", "plural_name": "Tokens",
                   "precision": 2, "product": "api", "prices": {"EUR": "0.01"}}),
        )
        .await;
    assert_eq!(unit.status, StatusCode::CREATED, "{:#}", unit.body);
    assert!(unit.body["id"].as_str().unwrap().starts_with("cun_"));
    assert_eq!(unit.body["product"]["code"], "api");
    assert!(
        unit.body["product"]["id"]
            .as_str()
            .unwrap()
            .starts_with("prd_")
    );
    assert_eq!(unit.body["prices"], json!({"EUR": "0.01"}));
    let bad = f
        .post(
            "/api/v2/credit-units",
            json!({"code": "EUR", "name": "X", "plural_name": "Xs", "precision": 0}),
        )
        .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad.body["errors"][0]["path"], "/code");
    // A unit is created with its prices or not at all: corrected, it is created under its code.
    let refused = f
        .post(
            "/api/v2/credit-units",
            json!({"code": "SEATS", "name": "Seat", "plural_name": "Seats", "precision": 0,
                   "prices": {"EUR": "1", "USD": "-1"}}),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused.body
    );
    assert_eq!(refused.body["errors"][0]["path"], "/prices/USD");
    assert_eq!(
        f.get("/api/v2/credit-units/SEATS").await.status,
        StatusCode::NOT_FOUND,
        "nothing half written"
    );
    let corrected = f
        .post(
            "/api/v2/credit-units",
            json!({"code": "SEATS", "name": "Seat", "plural_name": "Seats", "precision": 0,
                   "prices": {"EUR": "1", "USD": "1"}}),
        )
        .await;
    assert_eq!(
        corrected.status,
        StatusCode::CREATED,
        "{:#}",
        corrected.body
    );
    assert_eq!(corrected.body["prices"], json!({"EUR": "1", "USD": "1"}));

    let read = f.get("/api/v2/credit-units/TOKENS").await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["precision"], 2);
    let patched = f
        .patch(
            "/api/v2/credit-units/TOKENS",
            json!({"symbol": "TK", "prices": {"EUR": "0.02", "USD": "0.03"}}),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["symbol"], "TK");
    assert_eq!(
        patched.body["prices"],
        json!({"EUR": "0.02", "USD": "0.03"})
    );
    let cleared = f
        .patch(
            "/api/v2/credit-units/TOKENS",
            json!({"symbol": null, "prices": {"USD": null}}),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{:#}", cleared.body);
    assert!(cleared.body.get("symbol").is_none());
    assert_eq!(cleared.body["prices"], json!({"EUR": "0.02"}));

    let archived_unit = f
        .post("/api/v2/credit-units/TOKENS/archive", json!({}))
        .await;
    assert_eq!(
        archived_unit.status,
        StatusCode::OK,
        "{:#}",
        archived_unit.body
    );
    assert_eq!(archived_unit.body["archived"], true);
    let units = f.get("/api/v2/credit-units").await;
    assert!(!listed(&units).iter().any(|u| u["code"] == "TOKENS"));
    let all_units = f.get("/api/v2/credit-units?include_archived=true").await;
    assert!(listed(&all_units).iter().any(|u| u["code"] == "TOKENS"));
    let restored = f
        .post("/api/v2/credit-units/TOKENS/unarchive", json!({}))
        .await;
    assert_eq!(restored.body["archived"], false);
}

/// Rules: ADR-0013 §3 (archive), §7 (product families, managed quantities, rate cards)
#[tokio::test]
async fn test_rest_v2_families_quantities_and_rate_cards_archived() {
    let f = fixture().await;
    seed_catalog(&f).await;
    let refused_at = |answer: &Answer, suffix: &str| {
        assert_eq!(
            answer.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{:#}",
            answer.body
        );
        let error = &answer.body["errors"][0];
        assert_eq!(error["code"], "archived", "{:#}", answer.body);
        assert!(
            error["path"].as_str().unwrap().ends_with(suffix),
            "{suffix}: {:#}",
            answer.body
        );
    };

    // A product family: archived twice to the same answer, left out of the live list, still read.
    let family = f
        .post(
            "/api/v2/product-families",
            json!({"code": "legacy", "name": "Legacy"}),
        )
        .await;
    assert_eq!(family.status, StatusCode::CREATED, "{:#}", family.body);
    assert_eq!(family.body["archived"], false);
    let family_id = family.body["id"].as_str().unwrap().to_owned();
    for _ in 0..2 {
        let archived = f
            .post("/api/v2/product-families/legacy/archive", json!({}))
            .await;
        assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
        assert_eq!(archived.body["archived"], true);
    }
    let live = f.get("/api/v2/product-families?limit=100").await;
    assert!(!listed(&live).iter().any(|x| x["id"] == family_id.as_str()));
    let all = f
        .get("/api/v2/product-families?include_archived=true&limit=100")
        .await;
    assert!(listed(&all).iter().any(|x| x["id"] == family_id.as_str()));
    let read = f.get("/api/v2/product-families/legacy").await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["archived"], true);

    // Nothing new goes in it; restored, it takes products again.
    let product = json!({"code": "relic", "name": "Relic", "family": "legacy"});
    refused_at(
        &f.post("/api/v2/products", product.clone()).await,
        "/family",
    );
    let plan = f
        .post(
            "/api/v2/plans",
            json!({"code": "relic-plan", "name": "Relic", "family": "legacy",
                   "draft": {"charges": [{"product": "platform", "preset": "flat_fee", "prices": [
                       {"currency": "EUR", "cadence": "monthly",
                        "model": {"type": "flat", "amount": "5"}}]}]}}),
        )
        .await;
    refused_at(&plan, "/family");
    let restored = f
        .post("/api/v2/product-families/legacy/unarchive", json!({}))
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{:#}", restored.body);
    assert_eq!(restored.body["archived"], false);
    let created = f.post("/api/v2/products", product).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    assert_eq!(created.body["family"]["code"], "legacy");

    // A managed quantity counted by a plan, then archived: the plan still reads it, a new charge
    // counting it is refused.
    let quantity = f
        .post(
            "/api/v2/managed-quantities",
            json!({"code": "editors", "name": "Editors"}),
        )
        .await;
    assert_eq!(quantity.status, StatusCode::CREATED, "{:#}", quantity.body);
    let seats = |code: &str| {
        json!({"code": code, "name": "Editors plan", "draft": {"charges": [{
            "product": "seat", "preset": "seats",
            "quantity": {"type": "managed", "quantity": "editors"},
            "prices": [{"currency": "EUR", "model": {"type": "per_unit", "unit_price": "8"}}]
        }]}})
    };
    let counting = f.post("/api/v2/plans", seats("editors-plan")).await;
    assert_eq!(counting.status, StatusCode::CREATED, "{:#}", counting.body);
    for _ in 0..2 {
        let archived = f
            .post("/api/v2/managed-quantities/editors/archive", json!({}))
            .await;
        assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
        assert_eq!(archived.body["archived"], true);
    }
    let live = f.get("/api/v2/managed-quantities?limit=100").await;
    assert!(!listed(&live).iter().any(|q| q["code"] == "editors"));
    let all = f
        .get("/api/v2/managed-quantities?include_archived=true&limit=100")
        .await;
    assert!(listed(&all).iter().any(|q| q["code"] == "editors"));
    let kept = f.get("/api/v2/plans/editors-plan").await;
    assert_eq!(kept.status, StatusCode::OK, "{:#}", kept.body);
    assert_eq!(
        charge(&kept.body["version"], "seat")["quantity"]["quantity"],
        "editors"
    );
    refused_at(
        &f.post("/api/v2/plans", seats("editors-again")).await,
        "/draft/charges/0/quantity/quantity",
    );
    let per_editor = json!({"code": "exports-per-editor", "name": "Exports",
        "type": {"type": "limit", "counted_by": "app", "per_unit_of": "editors"}});
    refused_at(
        &f.post("/api/v2/features", per_editor.clone()).await,
        "/type/per_unit_of",
    );
    let restored = f
        .post("/api/v2/managed-quantities/editors/unarchive", json!({}))
        .await;
    assert_eq!(restored.body["archived"], false);
    let again = f.post("/api/v2/plans", seats("editors-again")).await;
    assert_eq!(again.status, StatusCode::CREATED, "{:#}", again.body);
    let counted = f.post("/api/v2/features", per_editor).await;
    assert_eq!(counted.status, StatusCode::CREATED, "{:#}", counted.body);
    assert_eq!(counted.body["type"]["per_unit_of"], "editors");

    // A rate card pricing a plan's usage, then archived: the plan reads it by code, its rates can
    // still be set, a new price by it is refused; each step is in its changes.
    let metric = f
        .post(
            "/api/v2/metrics",
            json!({"code": "arch_tokens", "name": "Tokens", "aggregation": "sum",
                   "aggregation_key": "tokens"}),
        )
        .await;
    assert_eq!(metric.status, StatusCode::CREATED, "{:#}", metric.body);
    let card = f
        .post(
            "/api/v2/rate-cards",
            json!({"code": "arch-rates", "name": "Rates", "metric": "arch_tokens",
                   "dimensions": ["model"]}),
        )
        .await;
    assert_eq!(card.status, StatusCode::CREATED, "{:#}", card.body);
    assert_eq!(card.body["archived"], false);
    let rated = |code: &str| {
        json!({"code": code, "name": "Rated", "draft": {"charges": [{
            "product": "api",
            "quantity": {"type": "metered", "metric": "arch_tokens"},
            "prices": [{"currency": "EUR",
                        "model": {"type": "rate_card", "rate_card": "arch-rates"}}]
        }]}})
    };
    let pricing = f.post("/api/v2/plans", rated("rated-plan")).await;
    assert_eq!(pricing.status, StatusCode::CREATED, "{:#}", pricing.body);
    for _ in 0..2 {
        let archived = f
            .post("/api/v2/rate-cards/arch-rates/archive", json!({}))
            .await;
        assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
        assert_eq!(archived.body["archived"], true);
    }
    let live = f.get("/api/v2/rate-cards?limit=100").await;
    assert!(!listed(&live).iter().any(|c| c["code"] == "arch-rates"));
    let all = f
        .get("/api/v2/rate-cards?include_archived=true&limit=100")
        .await;
    assert!(listed(&all).iter().any(|c| c["code"] == "arch-rates"));
    let kept = f.get("/api/v2/plans/rated-plan").await;
    assert_eq!(kept.status, StatusCode::OK, "{:#}", kept.body);
    assert_eq!(
        charge(&kept.body["version"], "api")["prices"][0]["model"]["rate_card"],
        "arch-rates",
        "an archived card is read by its code"
    );
    let set = f
        .patch(
            "/api/v2/rate-cards/arch-rates/rates",
            json!({"rates": [{"key": ["fast"], "currency": "EUR", "rate": "3"}]}),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{:#}", set.body);
    refused_at(
        &f.post("/api/v2/plans", rated("rated-again")).await,
        "/draft/charges/0/prices/0/model/rate_card",
    );
    let restored = f
        .post("/api/v2/rate-cards/arch-rates/unarchive", json!({}))
        .await;
    assert_eq!(restored.body["archived"], false);
    let changes = f.get("/api/v2/rate-cards/arch-rates/changes").await;
    let kinds: Vec<&str> = listed(&changes)
        .iter()
        .map(|change| change["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec!["unarchived", "rates_set", "archived", "created"],
        "archiving twice records it once"
    );
    let again = f.post("/api/v2/plans", rated("rated-again")).await;
    assert_eq!(again.status, StatusCode::CREATED, "{:#}", again.body);
}

/// The key's claim as a process that died leaves it: no response, claimed `minutes` ago.
async fn left_unanswered(f: &Fixture, key: &str, minutes: i32) -> usize {
    use diesel_async::RunQueryDsl;
    diesel::sql_query(
        "UPDATE api_idempotency_key SET response_status = NULL, response_body = NULL, \
         response_content_type = NULL, created_at = now() - make_interval(mins => $1) \
         WHERE operation = 'v2' AND idempotency_key = $2",
    )
    .bind::<diesel::sql_types::Integer, _>(minutes)
    .bind::<diesel::sql_types::Text, _>(key)
    .execute(&mut f.setup.store.get_conn().await.unwrap())
    .await
    .unwrap()
}
