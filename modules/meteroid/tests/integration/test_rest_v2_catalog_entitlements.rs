//! API v2's catalog entitlements (ADR-0013 §23): features of each type and their patches, a
//! row's lineage and history, feature statuses and codes, and the rows plans, add-ons and
//! products hold; what a plan is sold for and whom a metric's usage is attributed to.
use crate::test_rest_v2::{
    Answer, Fixture, fixture, granted, listed, refused, row_id, seed_catalog,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common_domain::ids::{BaseId, EntitlementId, FeatureId};
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::str::FromStr;

fn ok(answer: Answer) -> Value {
    assert_eq!(answer.status, StatusCode::OK, "{:#}", answer.body);
    answer.body
}

async fn created(f: &Fixture, path: &str, body: Value) -> Value {
    let answer = f.post(path, body).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "{path}: {:#}",
        answer.body
    );
    answer.body
}

fn limit(n: &str) -> Value {
    json!({"type": "limit", "limit": n})
}

fn on() -> Value {
    json!({"type": "on"})
}

/// A row of a feature applying per customer, as it reads back.
fn grant(feature: &str, value: Value, effect: &str) -> Value {
    json!({"feature": feature, "value": value, "effect": effect, "level": "customer"})
}

fn row_of<'a>(rows: &'a Value, feature: &str) -> &'a Value {
    rows.as_array()
        .and_then(|rows| rows.iter().find(|row| row["feature"] == feature))
        .unwrap_or_else(|| panic!("a row of {feature}: {rows:#}"))
}

/// The `site` resource type, with an `environment` attribute.
async fn site_type(f: &Fixture) {
    created(
        f,
        "/api/v2/resource-types",
        json!({"code": "site", "name": "Site", "plural_name": "Sites", "keys": ["site_id"],
               "attributes": {"environment": ["production", "staging"]}}),
    )
    .await;
}

async fn history(f: &Fixture, id: &str) -> Answer {
    f.get(&format!("/api/v2/entitlements/{id}/history")).await
}

async fn versions(f: &Fixture, id: &str) -> Vec<Value> {
    let answer = history(f, id).await;
    assert_eq!(answer.status, StatusCode::OK, "{id}: {:#}", answer.body);
    listed(&answer).clone()
}

async fn feature_codes(f: &Fixture, query: &str) -> Vec<String> {
    let answer = f.get(&format!("/api/v2/features{query}")).await;
    assert_eq!(answer.status, StatusCode::OK, "{query}: {:#}", answer.body);
    listed(&answer)
        .iter()
        .map(|feature| feature["code"].as_str().unwrap().to_owned())
        .collect()
}

/// Rules: ADR-0013 §23 (a feature)
#[tokio::test]
async fn test_rest_v2_features_of_each_type() {
    let f = fixture().await;
    seed_catalog(&f).await;
    site_type(&f).await;
    let policy = |resets: Value, at_limit: &str| json!({"resets": resets, "at_limit": at_limit, "grace_pct": 0, "thresholds": []});

    // Each type as written and read; a default of its value type; a limit's policy always read.
    let cases = [
        (
            json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"},
                   "defaults": [{"feature": "sso", "value": {"type": "on"}}]}),
            json!({"type": "boolean"}),
            on(),
        ),
        (
            json!({"code": "seats-max", "name": "Seats", "unit": "seats",
                   "type": {"type": "value", "value_type": "number"},
                   "defaults": [{"feature": "seats-max", "value": {"type": "number", "number": "30.0"}}]}),
            json!({"type": "value", "value_type": "number", "higher_is_better": true}),
            json!({"type": "number", "number": "30"}),
        ),
        (
            json!({"code": "retention", "name": "Retention",
                   "type": {"type": "value", "value_type": "number", "higher_is_better": false},
                   "defaults": [{"feature": "retention", "value": {"type": "number", "number": "7"}}]}),
            json!({"type": "value", "value_type": "number", "higher_is_better": false}),
            json!({"type": "number", "number": "7"}),
        ),
        (
            json!({"code": "region", "name": "Region",
                   "type": {"type": "value", "value_type": "text"},
                   "defaults": [{"feature": "region", "value": {"type": "text", "text": "eu"}}]}),
            json!({"type": "value", "value_type": "text"}),
            json!({"type": "text", "text": "eu"}),
        ),
        (
            json!({"code": "tier", "name": "Tier",
                   "type": {"type": "value", "value_type": "select",
                            "options": ["bronze", "silver", "gold"], "ordered": true},
                   "defaults": [{"feature": "tier", "value": {"type": "text", "text": "silver"}}]}),
            json!({"type": "value", "value_type": "select",
                   "options": ["bronze", "silver", "gold"], "ordered": true}),
            json!({"type": "text", "text": "silver"}),
        ),
        (
            json!({"code": "color", "name": "Color",
                   "type": {"type": "value", "value_type": "select", "options": ["red", "blue"]},
                   "defaults": [{"feature": "color", "value": {"type": "text", "text": "red"}}]}),
            json!({"type": "value", "value_type": "select", "options": ["red", "blue"],
                   "ordered": false}),
            json!({"type": "text", "text": "red"}),
        ),
        (
            json!({"code": "settings", "name": "Settings",
                   "type": {"type": "value", "value_type": "json"},
                   "defaults": [{"feature": "settings",
                                 "value": {"type": "json", "json": {"theme": "dark"}}}]}),
            json!({"type": "value", "value_type": "json"}),
            json!({"type": "json", "json": {"theme": "dark"}}),
        ),
        (
            json!({"code": "api-calls", "name": "API calls",
                   "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"},
                   "defaults": [{"feature": "api-calls", "value": {"type": "limit", "limit": "1000"}}]}),
            json!({"type": "limit", "counted_by": "events", "metric": "bandwidth",
                   "policy": policy(json!({"type": "billing_cycle"}), "deny")}),
            limit("1000"),
        ),
        (
            json!({"code": "projects", "name": "Projects", "unit": "projects",
                   "type": {"type": "limit", "counted_by": "app"},
                   "defaults": [{"feature": "projects", "value": {"type": "unlimited"}}]}),
            json!({"type": "limit", "counted_by": "app",
                   "policy": policy(json!({"type": "never"}), "deny_new")}),
            json!({"type": "unlimited"}),
        ),
        (
            json!({"code": "bursts", "name": "Bursts",
                   "type": {"type": "limit", "counted_by": "events", "metric": "db_size",
                            "policy": {"resets": {"type": "calendar", "unit": "month", "interval": 1},
                                       "at_limit": "deny", "grace_pct": 10,
                                       "thresholds": [90, 50]}},
                   "defaults": [{"feature": "bursts", "value": {"type": "off"}}]}),
            json!({"type": "limit", "counted_by": "events", "metric": "db_size",
                   "policy": {"resets": {"type": "calendar", "unit": "month", "interval": 1},
                              "at_limit": "deny", "grace_pct": 10, "thresholds": [50, 90]}}),
            json!({"type": "off"}),
        ),
    ];
    for (body, kind, value) in cases {
        let code = body["code"].as_str().unwrap().to_owned();
        let feature = created(&f, "/api/v2/features", body).await;
        assert_eq!(feature["type"], kind, "{code}");
        assert_eq!(feature["status"], "active", "{code}");
        assert_eq!(feature["defaults"][0]["value"], value, "{code}");
        assert_eq!(feature["defaults"][0]["effect"], "set", "{code}");
        let read = ok(f.get(&format!("/api/v2/features/{code}")).await);
        assert_eq!(read, feature, "{code}: read as written");
    }
    assert_eq!(
        ok(f.get("/api/v2/features/seats-max").await)["unit"],
        "seats"
    );
    assert_eq!(
        ok(f.get("/api/v2/features/projects").await)["unit"],
        "projects"
    );

    // A limit per unit of a managed quantity; a feature applying per a resource type.
    let per_user = created(
        &f,
        "/api/v2/features",
        json!({"code": "projects-per-user", "name": "Projects per user",
               "type": {"type": "limit", "counted_by": "app", "per_unit_of": "users"}}),
    )
    .await;
    assert_eq!(per_user["type"]["per_unit_of"], "users");
    let per_site = created(
        &f,
        "/api/v2/features",
        json!({"code": "pages", "name": "Pages", "type": {"type": "boolean"},
               "applies_per": "site"}),
    )
    .await;
    assert_eq!(per_site["applies_per"]["code"], "site");
    assert!(
        per_site["applies_per"]["id"]
            .as_str()
            .unwrap()
            .starts_with("rty_")
    );
    assert_eq!(per_site["defaults"], json!([]));

    // Refused at the pointer, nothing written.
    let deny_new = json!({"resets": {"type": "never"}, "at_limit": "deny_new"});
    let boolean = json!({"type": "boolean"});
    let app = json!({"type": "limit", "counted_by": "app"});
    let select = json!({"type": "value", "value_type": "select", "options": ["a", "b"]});
    let unordered_per_site = |rows: Value| {
        json!({"code": "flag", "name": "Flag", "type": select, "applies_per": "site",
               "defaults": rows})
    };
    let flag = |kind: &Value, extra: Value| {
        let mut body = json!({"code": "flag", "name": "Flag", "type": kind});
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };
    let refusals = [
        (
            json!({"code": "Bad Code", "name": "Bad", "type": boolean}),
            "/code",
            "invalid",
        ),
        (flag(&boolean, json!({"name": " "})), "/name", "required"),
        (flag(&boolean, json!({"unit": "flags"})), "/unit", "invalid"),
        (
            flag(&json!({"type": "value", "value_type": "select"}), json!({})),
            "/type/options",
            "required",
        ),
        (
            flag(
                &json!({"type": "value", "value_type": "select", "options": ["a", "a"]}),
                json!({}),
            ),
            "/type/options/1",
            "duplicate",
        ),
        (
            flag(
                &json!({"type": "value", "value_type": "text", "higher_is_better": true}),
                json!({}),
            ),
            "/type/higher_is_better",
            "invalid",
        ),
        (
            flag(
                &json!({"type": "value", "value_type": "text", "options": ["a"]}),
                json!({}),
            ),
            "/type/options",
            "invalid",
        ),
        (
            flag(&json!({"type": "limit", "counted_by": "events"}), json!({})),
            "/type/metric",
            "required",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "app", "metric": "bandwidth"}),
                json!({}),
            ),
            "/type/metric",
            "invalid",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "events", "metric": "nope"}),
                json!({}),
            ),
            "/type/metric",
            "not_found",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "events", "metric": "bandwidth",
                        "policy": deny_new}),
                json!({}),
            ),
            "/type/policy",
            "invalid",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "app",
                        "policy": {"resets": {"type": "billing_cycle"}, "at_limit": "deny"}}),
                json!({}),
            ),
            "/type/policy",
            "invalid",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "events", "metric": "bandwidth",
                        "policy": {"resets": {"type": "billing_cycle"},
                                   "at_limit": "allow_and_bill", "grace_pct": 5}}),
                json!({}),
            ),
            "/type/policy",
            "invalid",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "events", "metric": "bandwidth",
                        "policy": {"resets": {"type": "calendar", "unit": "month", "interval": 0},
                                   "at_limit": "deny"}}),
                json!({}),
            ),
            "/type/policy",
            "invalid",
        ),
        (
            flag(
                &json!({"type": "limit", "counted_by": "app", "per_unit_of": "nope"}),
                json!({}),
            ),
            "/type/per_unit_of",
            "not_found",
        ),
        (
            flag(&boolean, json!({"applies_per": "nope"})),
            "/applies_per",
            "not_found",
        ),
        (
            flag(&boolean, json!({"product": "nope"})),
            "/product",
            "not_found",
        ),
        (
            flag(
                &boolean,
                json!({"defaults": [{"feature": "sso", "value": on()}]}),
            ),
            "/defaults/0/feature",
            "invalid",
        ),
        (
            flag(
                &boolean,
                json!({"defaults": [{"feature": "flag", "value": on()},
                                    {"feature": "flag", "value": {"type": "off"}}]}),
            ),
            "/defaults/1/feature",
            "duplicate",
        ),
        (
            flag(
                &boolean,
                json!({"defaults": [{"feature": "flag", "value": on(), "effect": "raise_to"}]}),
            ),
            "/defaults/0/effect",
            "invalid",
        ),
        (
            flag(
                &boolean,
                json!({"defaults": [{"feature": "flag", "value": on(),
                                     "policy": {"resets": {"type": "never"}, "at_limit": "deny"}}]}),
            ),
            "/defaults/0/policy",
            "invalid",
        ),
        (
            flag(
                &boolean,
                json!({"defaults": [{"feature": "flag", "value": on(), "resource_type": "site"}]}),
            ),
            "/defaults/0/resource_type",
            "invalid",
        ),
        (
            flag(
                &boolean,
                json!({"applies_per": "site",
                       "defaults": [{"feature": "flag", "value": on(), "level": "resource_type"}]}),
            ),
            "/defaults/0/resource_type",
            "required",
        ),
        (
            flag(
                &boolean,
                json!({"defaults": [{"feature": "flag", "value": on(),
                                     "valid_from": "2027-02-01T00:00:00Z",
                                     "valid_until": "2027-01-01T00:00:00Z"}]}),
            ),
            "/defaults/0/valid_until",
            "invalid",
        ),
        (
            flag(
                &app,
                json!({"defaults": [{"feature": "flag", "value": limit("-1")}]}),
            ),
            "/defaults/0/value",
            "invalid",
        ),
        (
            flag(
                &select,
                json!({"defaults": [{"feature": "flag", "value": {"type": "text", "text": "c"}}]}),
            ),
            "/defaults/0/value",
            "invalid",
        ),
        (
            unordered_per_site(json!([{"feature": "flag",
                                       "value": {"type": "text", "text": "a"},
                                       "level": "customer"}])),
            "/defaults/0/level",
            "invalid",
        ),
    ];
    for (body, path, code) in refusals {
        refused(&f.post("/api/v2/features", body).await, path, code);
    }
    assert_eq!(
        f.get("/api/v2/features/flag").await.status,
        StatusCode::NOT_FOUND,
        "nothing written"
    );
}

/// Rules: ADR-0013 §23 (`PATCH /features/{…}`)
#[tokio::test]
async fn test_rest_v2_feature_patch() {
    let f = fixture().await;
    seed_catalog(&f).await;
    site_type(&f).await;
    for body in [
        json!({"code": "api-calls", "name": "API calls", "product": "api",
               "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"}}),
        json!({"code": "retention", "name": "Retention",
               "type": {"type": "value", "value_type": "number"}}),
        json!({"code": "tier", "name": "Tier",
               "type": {"type": "value", "value_type": "select",
                        "options": ["bronze", "silver", "gold"]},
               "defaults": [{"feature": "tier", "value": {"type": "text", "text": "silver"}}]}),
        json!({"code": "projects", "name": "Projects", "type": {"type": "limit", "counted_by": "app"}}),
        json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"},
               "defaults": [{"feature": "sso", "value": on()}]}),
        json!({"code": "pages", "name": "Pages", "type": {"type": "boolean"},
               "applies_per": "site"}),
    ] {
        created(&f, "/api/v2/features", body).await;
    }
    let patch = |code: &str| format!("/api/v2/features/{code}");

    // The type, the value type, the counting and the metric are fixed; sent as they are, they pass.
    for (code, body, path) in [
        (
            "api-calls",
            json!({"type": {"type": "boolean"}}),
            "/type/type",
        ),
        (
            "api-calls",
            json!({"type": {"counted_by": "app"}}),
            "/type/counted_by",
        ),
        (
            "api-calls",
            json!({"type": {"metric": "db_size"}}),
            "/type/metric",
        ),
        (
            "retention",
            json!({"type": {"value_type": "text"}}),
            "/type/value_type",
        ),
        ("sso", json!({"type": {"type": "limit"}}), "/type/type"),
        (
            "sso",
            json!({"type": {"value_type": "number"}}),
            "/type/value_type",
        ),
    ] {
        refused(&f.patch(&patch(code), body).await, path, "invalid");
    }
    let same = ok(f
        .patch(
            &patch("api-calls"),
            json!({"type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"}}),
        )
        .await);
    assert_eq!(same["type"]["metric"], "bandwidth");
    refused(
        &f.patch(&patch("api-calls"), json!({"type": {"metric": "nope"}}))
            .await,
        "/type/metric",
        "not_found",
    );

    // What a type changes: a number's direction, a select's options, a limit's policy.
    let lower = ok(f
        .patch(
            &patch("retention"),
            json!({"type": {"higher_is_better": false}}),
        )
        .await);
    assert_eq!(lower["type"]["higher_is_better"], false);
    refused(
        &f.patch(&patch("retention"), json!({"type": {"options": ["a"]}}))
            .await,
        "/type/options",
        "invalid",
    );
    refused(
        &f.patch(
            &patch("api-calls"),
            json!({"type": {"higher_is_better": true}}),
        )
        .await,
        "/type/higher_is_better",
        "invalid",
    );
    refused(
        &f.patch(
            &patch("api-calls"),
            json!({"type": {"policy": {"resets": {"type": "never"}, "at_limit": "deny_new"}}}),
        )
        .await,
        "/type/policy",
        "invalid",
    );
    refused(
        &f.patch(
            &patch("tier"),
            json!({"type": {"options": ["bronze", "gold"]}}),
        )
        .await,
        "/type/options",
        "invalid",
    );
    let more = ok(f
        .patch(
            &patch("tier"),
            json!({"type": {"options": ["bronze", "silver", "gold", "platinum"], "ordered": true}}),
        )
        .await);
    assert_eq!(
        more["type"]["options"],
        json!(["bronze", "silver", "gold", "platinum"])
    );
    assert_eq!(more["type"]["ordered"], true);
    assert_eq!(more["defaults"][0]["value"]["text"], "silver");

    // A limit counted per unit of a quantity, then per subscription again.
    let per_user = ok(f
        .patch(
            &patch("projects"),
            json!({"type": {"per_unit_of": "users"}}),
        )
        .await);
    assert_eq!(per_user["type"]["per_unit_of"], "users");
    let per_subscription = ok(f
        .patch(&patch("projects"), json!({"type": {"per_unit_of": null}}))
        .await);
    assert!(
        per_subscription["type"].get("per_unit_of").is_none(),
        "{per_subscription:#}"
    );
    refused(
        &f.patch(&patch("sso"), json!({"type": {"per_unit_of": "users"}}))
            .await,
        "/type/per_unit_of",
        "invalid",
    );
    refused(
        &f.patch(&patch("projects"), json!({"type": {"per_unit_of": "nope"}}))
            .await,
        "/type/per_unit_of",
        "not_found",
    );

    // The unit, the product, the name.
    let unit = ok(f
        .patch(&patch("api-calls"), json!({"unit": "requests"}))
        .await);
    assert_eq!(unit["unit"], "requests");
    let no_unit = ok(f.patch(&patch("api-calls"), json!({"unit": null})).await);
    assert!(no_unit.get("unit").is_none());
    refused(
        &f.patch(&patch("sso"), json!({"unit": "seats"})).await,
        "/unit",
        "invalid",
    );
    let moved = ok(f.patch(&patch("sso"), json!({"product": "platform"})).await);
    assert_eq!(moved["product"]["code"], "platform");
    let everyone = ok(f.patch(&patch("sso"), json!({"product": null})).await);
    assert!(everyone.get("product").is_none());
    refused(
        &f.patch(&patch("sso"), json!({"product": "nope"})).await,
        "/product",
        "not_found",
    );
    refused(
        &f.patch(&patch("sso"), json!({"name": " "})).await,
        "/name",
        "invalid",
    );

    // The level a feature applies per changes only while it has no rows.
    refused(
        &f.patch(&patch("sso"), json!({"applies_per": "site"})).await,
        "/applies_per",
        "invalid",
    );
    let emptied = ok(f.patch(&patch("sso"), json!({"defaults": []})).await);
    assert_eq!(emptied["defaults"], json!([]));
    let per_site = ok(f.patch(&patch("sso"), json!({"applies_per": "site"})).await);
    assert_eq!(per_site["applies_per"]["code"], "site");
    let per_customer = ok(f.patch(&patch("sso"), json!({"applies_per": null})).await);
    assert!(per_customer.get("applies_per").is_none());
    refused(
        &f.patch(&patch("sso"), json!({"applies_per": "nope"})).await,
        "/applies_per",
        "not_found",
    );

    // Its fields and its defaults are written together: a default the store refuses (a
    // condition on a value its attribute does not declare) writes neither.
    let off_value = f
        .patch(
            &patch("pages"),
            json!({"name": "Site pages",
                   "defaults": [{"feature": "pages", "value": on(),
                                 "condition": {"site.environment": ["mars"]}}]}),
        )
        .await;
    refused(&off_value, "/defaults", "invalid");
    assert_eq!(ok(f.get(&patch("pages")).await)["name"], "Pages");
    let conditioned = ok(f
        .patch(
            &patch("pages"),
            json!({"name": "Site pages",
                   "defaults": [{"feature": "pages", "value": on(),
                                 "condition": {"site.environment": ["staging"]}}]}),
        )
        .await);
    assert_eq!(conditioned["name"], "Site pages");
    assert_eq!(
        conditioned["defaults"][0]["condition"],
        json!({"site.environment": ["staging"]})
    );

    // An archived feature is not patched, by its id or by the code it had.
    let archived = ok(f.post("/api/v2/features/sso/archive", json!({})).await);
    for target in ["sso", archived["id"].as_str().unwrap()] {
        let answer = f.patch(&patch(target), json!({"name": "SSO 2"})).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{:#}", answer.body);
        assert_eq!(answer.body["code"], "conflict");
    }
}

/// Rules: ADR-0013 §23 (an entitlement: lineage, `/entitlements/{id}`, its history)
#[tokio::test]
async fn test_rest_v2_feature_defaults_lineage_and_history() {
    let f = fixture().await;
    let feature = created(
        &f,
        "/api/v2/features",
        json!({"code": "projects", "name": "Projects",
               "type": {"type": "limit", "counted_by": "app"},
               "defaults": [{"feature": "projects", "value": limit("10"), "reason": "Launch"}]}),
    )
    .await;
    let feature_id = feature["id"].as_str().unwrap().to_owned();
    let default = feature["defaults"][0].clone();
    let id = row_id(&default);
    assert_eq!(default["reason"], "Launch");
    assert_eq!(default["created_by"]["type"], "api_token");
    assert!(default.get("revoked_at").is_none());
    let read = ok(f.get(&format!("/api/v2/entitlements/{id}")).await);
    assert_eq!(read, default, "any row reads by its id");
    let path = "/api/v2/features/projects";
    let id_of = |feature: &Value| row_id(&feature["defaults"][0]);

    // Sent the same, the row is kept as it is; at the customer level, a customer feature's row
    // is the same row (one spelling per key).
    let same = ok(f
        .patch(
            path,
            json!({"defaults": [{"feature": "projects", "value": limit("10"), "reason": "Launch"}]}),
        )
        .await);
    assert_eq!(id_of(&same), id);
    let spelled = ok(f
        .patch(
            path,
            json!({"defaults": [{"feature": feature_id, "value": limit("10.00"),
                                 "level": "customer", "reason": "Launch"}]}),
        )
        .await);
    assert_eq!(id_of(&spelled), id);
    assert_eq!(spelled["defaults"][0]["level"], "customer");
    let feature_spelled = f
        .patch(
            path,
            json!({"defaults": [{"feature": "projects", "value": limit("10"), "level": "feature"}]}),
        )
        .await;
    assert_eq!(
        feature_spelled.status,
        StatusCode::BAD_REQUEST,
        "{:#}",
        feature_spelled.body
    );
    assert_eq!(
        feature_spelled.body["errors"][0]["path"],
        "/defaults/0/level"
    );
    assert_eq!(versions(&f, &id).await.len(), 1, "no version written");

    // Changed, it is succeeded under its id: its versions read newest first.
    let raised = ok(f
        .patch(
            path,
            json!({"defaults": [{"feature": "projects", "value": limit("25")}]}),
        )
        .await);
    assert_eq!(id_of(&raised), id);
    let both = versions(&f, &id).await;
    let values: Vec<Value> = both
        .iter()
        .map(|version| version["value"].clone())
        .collect();
    assert_eq!(values, vec![limit("25"), limit("10")], "newest first");
    assert!(both[0].get("revoked_at").is_none());
    assert!(both[1]["revoked_at"].is_string(), "{:#}", both[1]);
    assert!(
        both[0].get("reason").is_none(),
        "a reason left out is cleared"
    );
    assert_eq!(both[1]["reason"], "Launch");
    assert_eq!(
        both[0]["created_at"], both[1]["created_at"],
        "the lineage's creation"
    );
    assert_eq!(
        ok(f.get(&format!("/api/v2/entitlements/{id}")).await)["value"],
        limit("25")
    );

    // Paged.
    let first = f
        .get(&format!("/api/v2/entitlements/{id}/history?limit=1"))
        .await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(listed(&first).len(), 1);
    assert_eq!(listed(&first)[0]["value"], limit("25"));
    assert_eq!(first.body["has_more"], true);
    let next = f
        .get(&format!(
            "/api/v2/entitlements/{id}/history?limit=1&after={}",
            first.body["next_cursor"].as_str().unwrap()
        ))
        .await;
    assert_eq!(next.status, StatusCode::OK, "{:#}", next.body);
    assert_eq!(listed(&next)[0]["value"], limit("10"));
    assert_eq!(next.body["has_more"], false);
    refused(
        &f.get(&format!("/api/v2/entitlements/{id}/history?after=garbage"))
            .await,
        "after",
        "invalid",
    );
    refused(
        &f.get(&format!("/api/v2/entitlements/{id}/history?limit=0"))
            .await,
        "limit",
        "invalid",
    );

    // Left out, it is revoked: no longer found, its history kept.
    let removed = ok(f.patch(path, json!({"defaults": []})).await);
    assert_eq!(removed["defaults"], json!([]));
    let gone = f.get(&format!("/api/v2/entitlements/{id}")).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{:#}", gone.body);
    let kept = versions(&f, &id).await;
    assert_eq!(kept.len(), 2);
    assert!(
        kept.iter().all(|version| version["revoked_at"].is_string()),
        "{kept:#?}"
    );

    // Neither a row never written nor an id of another kind is one.
    let unknown = EntitlementId::new().to_string();
    for path in [
        format!("/api/v2/entitlements/{unknown}"),
        format!("/api/v2/entitlements/{unknown}/history"),
        format!("/api/v2/entitlements/{feature_id}"),
        "/api/v2/entitlements/nope/history".to_owned(),
    ] {
        let answer = f.get(&path).await;
        assert_eq!(
            answer.status,
            StatusCode::NOT_FOUND,
            "{path}: {:#}",
            answer.body
        );
        assert_eq!(answer.body["code"], "not_found", "{path}");
    }
}

/// Rules: ADR-0013 §23 (`GET /features`, statuses, codes)
#[tokio::test]
async fn test_rest_v2_feature_statuses_codes_and_lists() {
    let f = fixture().await;
    seed_catalog(&f).await;
    site_type(&f).await;
    let mut ids: Vec<String> = Vec::new();
    for body in [
        json!({"code": "a-sso", "name": "SSO", "type": {"type": "boolean"}, "product": "platform"}),
        json!({"code": "b-calls", "name": "Calls", "product": "api",
               "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"}}),
        json!({"code": "c-pages", "name": "Pages", "type": {"type": "boolean"},
               "applies_per": "site"}),
        json!({"code": "d-beta", "name": "Beta", "type": {"type": "boolean"}}),
    ] {
        let feature = created(&f, "/api/v2/features", body).await;
        ids.push(feature["id"].as_str().unwrap().to_owned());
    }
    let beta_id = ids[3].clone();
    let mut by_id = ids.clone();
    by_id.sort_by_key(|id| FeatureId::from_str(id).unwrap().as_uuid());

    // Paged by keyset over the id, filtered.
    let first = f.get("/api/v2/features?limit=3").await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(listed(&first).len(), 3);
    assert_eq!(first.body["has_more"], true);
    let rest = f
        .get(&format!(
            "/api/v2/features?limit=3&after={}",
            first.body["next_cursor"].as_str().unwrap()
        ))
        .await;
    assert_eq!(rest.status, StatusCode::OK, "{:#}", rest.body);
    assert_eq!(rest.body["has_more"], false);
    let paged: Vec<String> = listed(&first)
        .iter()
        .chain(listed(&rest).iter())
        .map(|feature| feature["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(paged, by_id);
    assert_eq!(feature_codes(&f, "?product=api").await, vec!["b-calls"]);
    assert_eq!(
        feature_codes(&f, "?applies_per=site").await,
        vec!["c-pages"]
    );
    let per_customer = feature_codes(&f, "?applies_per=customer").await;
    assert_eq!(per_customer.len(), 3, "{per_customer:?}");
    assert!(!per_customer.contains(&"c-pages".to_owned()));
    refused(
        &f.get("/api/v2/features?product=nope").await,
        "product",
        "not_found",
    );
    refused(
        &f.get("/api/v2/features?applies_per=nope").await,
        "applies_per",
        "not_found",
    );
    refused(
        &f.get("/api/v2/features?status=gone").await,
        "status",
        "invalid",
    );
    refused(&f.get("/api/v2/features?limit=0").await, "limit", "invalid");
    refused(
        &f.get("/api/v2/features?after=garbage").await,
        "after",
        "invalid",
    );
    let stranger = URL_SAFE_NO_PAD.encode(FeatureId::new().as_uuid().to_string());
    refused(
        &f.get(&format!("/api/v2/features?after={stranger}")).await,
        "after",
        "invalid",
    );

    // Disabled: listed, entitling no one; disabling or enabling twice changes nothing.
    let disabled = ok(f.post("/api/v2/features/d-beta/disable", json!({})).await);
    assert_eq!(disabled["status"], "disabled");
    let again = ok(f.post("/api/v2/features/d-beta/disable", json!({})).await);
    assert_eq!(again, disabled);
    assert!(feature_codes(&f, "").await.contains(&"d-beta".to_owned()));
    assert_eq!(feature_codes(&f, "?status=disabled").await, vec!["d-beta"]);
    assert!(
        !feature_codes(&f, "?status=active")
            .await
            .contains(&"d-beta".to_owned())
    );
    assert_eq!(
        feature_codes(&f, "?status=active&status=disabled")
            .await
            .len(),
        4
    );
    let enabled = ok(f.post("/api/v2/features/d-beta/enable", json!({})).await);
    assert_eq!(enabled["status"], "active");
    assert_eq!(
        ok(f.post("/api/v2/features/d-beta/enable", json!({})).await),
        enabled
    );

    // Archived: left out unless asked, still read by its code; neither disabled nor enabled.
    let archived = ok(f.post("/api/v2/features/d-beta/archive", json!({})).await);
    assert_eq!(archived["status"], "archived");
    assert_eq!(
        ok(f.post("/api/v2/features/d-beta/archive", json!({})).await),
        archived
    );
    assert!(!feature_codes(&f, "").await.contains(&"d-beta".to_owned()));
    assert_eq!(feature_codes(&f, "?status=archived").await, vec!["d-beta"]);
    assert_eq!(ok(f.get("/api/v2/features/d-beta").await)["id"], beta_id);
    for action in ["disable", "enable"] {
        let answer = f
            .post(&format!("/api/v2/features/d-beta/{action}"), json!({}))
            .await;
        assert_eq!(
            answer.status,
            StatusCode::CONFLICT,
            "{action}: {:#}",
            answer.body
        );
        assert_eq!(answer.body["code"], "conflict");
    }
    // A row naming it is refused at the row.
    refused(
        &f.patch(
            "/api/v2/products/platform",
            json!({"entitlements": [{"feature": beta_id, "value": on()}]}),
        )
        .await,
        "/entitlements/0/feature",
        "archived",
    );

    // Its code is free: a new feature takes it, and the code names that one. Restoring the
    // first is refused while the code is held, by code or by id.
    let successor = created(
        &f,
        "/api/v2/features",
        json!({"code": "d-beta", "name": "Beta 2", "type": {"type": "boolean"}}),
    )
    .await;
    let successor_id = successor["id"].as_str().unwrap().to_owned();
    assert_ne!(successor_id, beta_id);
    assert_eq!(
        ok(f.get("/api/v2/features/d-beta").await)["id"],
        successor_id
    );
    for target in ["d-beta", beta_id.as_str()] {
        let answer = f
            .post(&format!("/api/v2/features/{target}/unarchive"), json!({}))
            .await;
        assert_eq!(
            answer.status,
            StatusCode::CONFLICT,
            "{target}: {:#}",
            answer.body
        );
        assert_eq!(answer.body["code"], "code_taken");
        assert_eq!(answer.body["errors"][0]["path"], "/code");
    }

    // Two archived features had the code: it names neither.
    ok(f.post(
        &format!("/api/v2/features/{successor_id}/archive"),
        json!({}),
    )
    .await);
    let ambiguous = f.get("/api/v2/features/d-beta").await;
    assert_eq!(
        ambiguous.status,
        StatusCode::CONFLICT,
        "{:#}",
        ambiguous.body
    );
    let restored = ok(f
        .post(&format!("/api/v2/features/{beta_id}/unarchive"), json!({}))
        .await);
    assert_eq!(restored["status"], "active");
    assert_eq!(restored["id"], beta_id);
    assert_eq!(ok(f.get("/api/v2/features/d-beta").await)["id"], beta_id);
    assert_eq!(
        ok(
            f.post(&format!("/api/v2/features/{beta_id}/unarchive"), json!({}))
                .await
        )["status"],
        "active"
    );
    assert_eq!(
        f.get("/api/v2/features/nope").await.status,
        StatusCode::NOT_FOUND
    );
}

/// A draft billing the platform, with `rows` when sent.
fn draft(rows: Option<Value>) -> Value {
    let mut draft = json!({"charges": [{"product": "platform", "preset": "flat_fee", "prices": [
        {"currency": "EUR", "cadence": "monthly", "model": {"type": "flat", "amount": "5"}}
    ]}]});
    if let Some(rows) = rows {
        draft["entitlements"] = rows;
    }
    draft
}

/// Rules: ADR-0013 §23 (a plan version's rows, `sold_for`)
#[tokio::test]
async fn test_rest_v2_plan_rows_and_sold_for() {
    let f = fixture().await;
    seed_catalog(&f).await;
    site_type(&f).await;
    for body in [
        json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"}}),
        json!({"code": "api-calls", "name": "API calls",
               "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"}}),
        json!({"code": "pages", "name": "Pages", "type": {"type": "limit", "counted_by": "app"},
               "applies_per": "site"}),
    ] {
        created(&f, "/api/v2/features", body).await;
    }
    let sso = json!({"feature": "sso", "value": on()});
    let sso_off = json!({"feature": "sso", "value": {"type": "off"}});
    let calls = json!({"feature": "api-calls", "value": limit("1000")});

    // Two rows of one key over the same dates: refused at the second, nothing written.
    let twice = f
        .post(
            "/api/v2/plans",
            json!({"code": "rows", "name": "Rows",
                   "draft": draft(Some(json!([sso, sso_off])))}),
        )
        .await;
    refused(&twice, "/draft/entitlements/1/feature", "duplicate");
    assert_eq!(
        f.get("/api/v2/plans/rows").await.status,
        StatusCode::NOT_FOUND
    );
    let plan = created(
        &f,
        "/api/v2/plans",
        json!({"code": "rows", "name": "Rows", "draft": draft(Some(json!([sso])))}),
    )
    .await;
    assert_eq!(
        granted(&plan["version"]["entitlements"]),
        vec![grant("sso", on(), "set")]
    );

    // Left out, the draft's rows are carried over; dated rows of one key may follow each other;
    // sent empty, they are cleared.
    let carried = ok(f.put("/api/v2/plans/rows/draft", draft(None)).await);
    assert_eq!(
        granted(&carried["version"]["entitlements"]),
        granted(&plan["version"]["entitlements"])
    );
    let dated = ok(f
        .put(
            "/api/v2/plans/rows/draft",
            draft(Some(json!([
                {"feature": "sso", "value": on(), "valid_until": "2027-01-01T00:00:00Z"},
                {"feature": "sso", "value": {"type": "off"}, "valid_from": "2027-01-01T00:00:00Z"}
            ]))),
        )
        .await);
    let rows = granted(&dated["version"]["entitlements"]);
    assert_eq!(rows.len(), 2, "{rows:#?}");
    assert_eq!(rows[0]["value"], on());
    assert!(rows[0].get("valid_from").is_none());
    assert_eq!(rows[0]["valid_until"], "2027-01-01T00:00:00.000Z");
    assert_eq!(rows[1]["value"], json!({"type": "off"}));
    assert_eq!(rows[1]["valid_from"], "2027-01-01T00:00:00.000Z");
    refused(
        &f.put(
            "/api/v2/plans/rows/draft",
            draft(Some(json!([sso, sso_off]))),
        )
        .await,
        "/entitlements/1/feature",
        "duplicate",
    );
    let cleared = ok(f
        .put("/api/v2/plans/rows/draft", draft(Some(json!([]))))
        .await);
    assert_eq!(cleared["version"]["entitlements"], json!([]));
    let still_cleared = ok(f.put("/api/v2/plans/rows/draft", draft(None)).await);
    assert_eq!(still_cleared["version"]["entitlements"], json!([]));

    // Published, a version's rows carry over to its next draft, which holds its own.
    ok(
        f.put("/api/v2/plans/rows/draft", draft(Some(json!([sso, calls]))))
            .await,
    );
    let published = ok(f.post("/api/v2/plans/rows/draft/publish", json!({})).await);
    let active_rows = published["version"]["entitlements"].clone();
    assert_eq!(granted(&active_rows).len(), 2, "{active_rows:#}");
    let active_row = row_id(&active_rows[0]);
    assert_eq!(
        ok(f.get(&format!("/api/v2/entitlements/{active_row}")).await)["owner"]["type"],
        "plan_version"
    );
    let next = ok(f.put("/api/v2/plans/rows/draft", draft(None)).await);
    assert_eq!(next["version"]["number"], 2);
    assert_eq!(
        granted(&next["version"]["entitlements"]),
        granted(&active_rows)
    );
    // A draft read is written back as is: its rows' own fields are taken and ignored.
    let again = ok(f
        .put("/api/v2/plans/rows/draft", next["version"].clone())
        .await);
    assert_eq!(
        granted(&again["version"]["entitlements"]),
        granted(&next["version"]["entitlements"])
    );
    assert_ne!(
        next["version"]["entitlements"][0]["owner"]["id"], active_rows[0]["owner"]["id"],
        "the draft's own rows"
    );

    // Discarded, a draft goes with its rows; the version keeps its own.
    let draft_row = row_id(&next["version"]["entitlements"][0]);
    assert_eq!(
        f.delete("/api/v2/plans/rows/draft").await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(history(&f, &draft_row).await.status, StatusCode::NOT_FOUND);
    let active = ok(f.get("/api/v2/plans/rows").await);
    assert_eq!(active["version"]["entitlements"], active_rows);
    assert_eq!(versions(&f, &active_row).await.len(), 1);
    // A plan never published goes with its draft, and so do its rows.
    let scratch = created(
        &f,
        "/api/v2/plans",
        json!({"code": "scratch", "name": "Scratch", "draft": draft(Some(json!([sso])))}),
    )
    .await;
    let scratch_row = row_id(&scratch["version"]["entitlements"][0]);
    assert_eq!(
        f.delete("/api/v2/plans/scratch/draft").await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.get("/api/v2/plans/scratch").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        history(&f, &scratch_row).await.status,
        StatusCode::NOT_FOUND
    );

    // What a plan is sold for: set on create and by a patch, `null` for the customer.
    let per_site = created(
        &f,
        "/api/v2/plans",
        json!({"code": "per-site", "name": "Per site", "sold_for": "site",
               "draft": draft(Some(json!([{"feature": "pages", "value": limit("10")}])))}),
    )
    .await;
    assert_eq!(per_site["sold_for"]["code"], "site");
    let plans = f.get("/api/v2/plans?limit=100").await;
    let listed_plan = listed(&plans)
        .iter()
        .find(|plan| plan["code"] == "per-site")
        .expect("per-site is listed");
    assert_eq!(listed_plan["sold_for"]["code"], "site");
    refused(
        &f.post(
            "/api/v2/plans",
            json!({"code": "nowhere", "name": "Nowhere", "sold_for": "nope", "draft": draft(None)}),
        )
        .await,
        "/sold_for",
        "not_found",
    );
    let for_customers = ok(f
        .patch("/api/v2/plans/per-site", json!({"sold_for": null}))
        .await);
    assert!(for_customers.get("sold_for").is_none(), "{for_customers:#}");
    let for_sites = ok(f
        .patch("/api/v2/plans/per-site", json!({"sold_for": "site"}))
        .await);
    assert_eq!(for_sites["sold_for"]["code"], "site");
    refused(
        &f.patch("/api/v2/plans/per-site", json!({"sold_for": "nope"}))
            .await,
        "/sold_for",
        "not_found",
    );

    // Sold for a type, a plan carries no rows across all its resources.
    let across = json!([{"feature": "pages", "value": limit("100"), "level": "customer"}]);
    let refused_create = f
        .post(
            "/api/v2/plans",
            json!({"code": "across", "name": "Across", "sold_for": "site",
                   "draft": draft(Some(across.clone()))}),
        )
        .await;
    assert_eq!(
        refused_create.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused_create.body
    );
    assert_eq!(
        f.get("/api/v2/plans/across").await.status,
        StatusCode::NOT_FOUND
    );
    let refused_draft = f
        .put("/api/v2/plans/per-site/draft", draft(Some(across.clone())))
        .await;
    assert_eq!(
        refused_draft.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        refused_draft.body
    );
    let wide = created(
        &f,
        "/api/v2/plans",
        json!({"code": "across", "name": "Across", "draft": draft(Some(across))}),
    )
    .await;
    assert_eq!(wide["version"]["entitlements"][0]["level"], "customer");
    assert!(wide.get("sold_for").is_none());
    refused(
        &f.patch("/api/v2/plans/across", json!({"sold_for": "site"}))
            .await,
        "/sold_for",
        "invalid",
    );
    assert!(
        ok(f.get("/api/v2/plans/across").await)
            .get("sold_for")
            .is_none(),
        "nothing written"
    );
}

/// Rules: ADR-0013 §23 (an add-on's and a product's rows)
#[tokio::test]
async fn test_rest_v2_add_on_and_product_rows() {
    let f = fixture().await;
    seed_catalog(&f).await;
    for body in [
        json!({"code": "api-calls", "name": "API calls", "product": "api",
               "type": {"type": "limit", "counted_by": "events", "metric": "bandwidth"}}),
        json!({"code": "projects", "name": "Projects", "product": "api",
               "type": {"type": "limit", "counted_by": "app"}}),
        json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"}, "product": "platform"}),
        json!({"code": "audit", "name": "Audit log", "type": {"type": "boolean"}}),
    ] {
        created(&f, "/api/v2/features", body).await;
    }
    let calls = |n: &str| json!({"feature": "api-calls", "value": limit(n)});
    let projects = |n: &str| json!({"feature": "projects", "value": limit(n)});
    let add_on = |code: &str, max_instances: Option<i32>, rows: Value| {
        json!({"code": code, "name": code, "max_instances": max_instances,
               "charge": {"product": "api", "preset": "flat_fee", "prices": [
                   {"currency": "EUR", "cadence": "monthly",
                    "model": {"type": "flat", "amount": "20"}}
               ]},
               "entitlements": rows})
    };

    // Bought any number of times, an add-on's rows add up per instance; bought once, they raise.
    let stacked = created(
        &f,
        "/api/v2/add-ons",
        add_on("more", None, json!([calls("1000"), projects("5")])),
    )
    .await;
    assert_eq!(
        granted(&stacked["entitlements"]),
        vec![
            grant("api-calls", limit("1000"), "add"),
            grant("projects", limit("5"), "add"),
        ]
    );
    assert_eq!(stacked["entitlements"][0]["owner"]["type"], "add_on");
    let once = created(
        &f,
        "/api/v2/add-ons",
        add_on("boost", Some(1), json!([calls("1000")])),
    )
    .await;
    assert_eq!(once["entitlements"][0]["effect"], "raise_to");
    let raising = created(
        &f,
        "/api/v2/add-ons",
        add_on(
            "raise",
            None,
            json!([{"feature": "api-calls", "value": limit("1"), "effect": "raise_to"}]),
        ),
    )
    .await;
    assert_eq!(raising["entitlements"][0]["effect"], "raise_to");
    refused(
        &f.post(
            "/api/v2/add-ons",
            add_on(
                "setter",
                None,
                json!([{"feature": "api-calls", "value": limit("1"), "effect": "set"}]),
            ),
        )
        .await,
        "/entitlements/0/effect",
        "invalid",
    );
    refused(
        &f.post(
            "/api/v2/add-ons",
            add_on("ghost", None, json!([{"feature": "nope", "value": on()}])),
        )
        .await,
        "/entitlements/0/feature",
        "not_found",
    );
    for code in ["setter", "ghost"] {
        assert_eq!(
            f.get(&format!("/api/v2/add-ons/{code}")).await.status,
            StatusCode::NOT_FOUND,
            "{code}: nothing written"
        );
    }

    // A patch matches rows by key: one sent the same keeps its version, one changed is
    // succeeded under its id, one left out is revoked with its history kept.
    let calls_row = row_id(row_of(&stacked["entitlements"], "api-calls"));
    let projects_row = row_id(row_of(&stacked["entitlements"], "projects"));
    let patched = ok(f
        .patch(
            "/api/v2/add-ons/more",
            json!({"entitlements": [calls("1000"), projects("10")]}),
        )
        .await);
    assert_eq!(
        row_id(row_of(&patched["entitlements"], "api-calls")),
        calls_row
    );
    assert_eq!(
        row_id(row_of(&patched["entitlements"], "projects")),
        projects_row
    );
    assert_eq!(
        row_of(&patched["entitlements"], "projects")["value"],
        limit("10")
    );
    assert_eq!(versions(&f, &calls_row).await.len(), 1, "kept as it is");
    assert_eq!(versions(&f, &projects_row).await.len(), 2, "succeeded");
    let fewer = ok(f
        .patch(
            "/api/v2/add-ons/more",
            json!({"entitlements": [projects("10")]}),
        )
        .await);
    assert_eq!(
        granted(&fewer["entitlements"]),
        vec![grant("projects", limit("10"), "add")]
    );
    assert_eq!(
        f.get(&format!("/api/v2/entitlements/{calls_row}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let revoked = versions(&f, &calls_row).await;
    assert_eq!(revoked.len(), 1);
    assert!(revoked[0]["revoked_at"].is_string(), "{revoked:#?}");
    assert_eq!(versions(&f, &projects_row).await.len(), 2);
    refused(
        &f.patch(
            "/api/v2/add-ons/more",
            json!({"entitlements": [{"feature": "api-calls", "value": limit("1"),
                                     "effect": "cap_at"}]}),
        )
        .await,
        "/entitlements/0/effect",
        "invalid",
    );
    assert_eq!(
        ok(f.get("/api/v2/add-ons/more").await)["entitlements"],
        fewer["entitlements"],
        "nothing written"
    );

    // A product holds its own features' defaults, replaced together; another product's feature
    // or a feature of every product is refused at its row.
    let product = ok(f
        .patch(
            "/api/v2/products/api",
            json!({"entitlements": [calls("1000"), projects("5")]}),
        )
        .await);
    assert_eq!(
        granted(&product["entitlements"]),
        vec![
            grant("api-calls", limit("1000"), "set"),
            grant("projects", limit("5"), "set"),
        ]
    );
    let calls_default = row_id(row_of(&product["entitlements"], "api-calls"));
    assert_eq!(
        row_of(&product["entitlements"], "api-calls")["owner"]["type"],
        "feature"
    );
    for stranger in ["sso", "audit"] {
        refused(
            &f.patch(
                "/api/v2/products/api",
                json!({"entitlements": [calls("1000"), {"feature": stranger, "value": on()}]}),
            )
            .await,
            "/entitlements/1/feature",
            "invalid",
        );
    }
    assert_eq!(
        granted(&ok(f.get("/api/v2/products/api").await)["entitlements"]),
        granted(&product["entitlements"]),
        "nothing written"
    );
    let narrowed = ok(f
        .patch(
            "/api/v2/products/api",
            json!({"entitlements": [calls("2000")]}),
        )
        .await);
    assert_eq!(
        granted(&narrowed["entitlements"]),
        vec![grant("api-calls", limit("2000"), "set")]
    );
    assert_eq!(row_id(&narrowed["entitlements"][0]), calls_default);
    assert_eq!(
        ok(f.get("/api/v2/features/projects").await)["defaults"],
        json!([]),
        "a feature of the product left out loses its defaults"
    );
}

/// Rules: ADR-0013 §23 (a metric's `attributed_to` and its preview)
#[tokio::test]
async fn test_rest_v2_metric_attribution() {
    let f = fixture().await;
    site_type(&f).await;
    created(
        &f,
        "/api/v2/resource-types",
        json!({"code": "legacy", "name": "Legacy", "plural_name": "Legacies",
               "keys": ["legacy_id"]}),
    )
    .await;
    ok(f.post("/api/v2/resource-types/legacy/archive", json!({}))
        .await);
    let metric = |code: &str, attributed_to: Option<Value>| {
        let mut body = json!({"code": code, "name": code, "aggregation": "count"});
        if let Some(attributed_to) = attributed_to {
            body["attributed_to"] = attributed_to;
        }
        body
    };
    let to_site = json!({"type": "resource_type", "resource_type": "site"});

    let per_site = created(
        &f,
        "/api/v2/metrics",
        metric("site_events", Some(to_site.clone())),
    )
    .await;
    assert_eq!(per_site["attributed_to"], to_site);
    let keyed = created(
        &f,
        "/api/v2/metrics",
        metric(
            "keyed_events",
            Some(json!({"type": "resource_type", "resource_type": "site", "property": "site_key"})),
        ),
    )
    .await;
    assert_eq!(
        keyed["attributed_to"],
        json!({"type": "resource_type", "resource_type": "site", "property": "site_key"})
    );
    let plain = created(&f, "/api/v2/metrics", metric("plain_events", None)).await;
    assert_eq!(plain["attributed_to"], json!({"type": "customer"}));

    // Refused at the pointer, nothing written.
    for (attributed_to, path, code) in [
        (
            json!({"type": "resource_type", "resource_type": "nope"}),
            "/attributed_to/resource_type",
            "not_found",
        ),
        (
            json!({"type": "resource_type", "resource_type": "legacy"}),
            "/attributed_to",
            "invalid",
        ),
        (
            json!({"type": "resource_type", "resource_type": "site", "property": " padded "}),
            "/attributed_to",
            "invalid",
        ),
    ] {
        refused(
            &f.post(
                "/api/v2/metrics",
                metric("refused_events", Some(attributed_to)),
            )
            .await,
            path,
            code,
        );
    }
    assert_eq!(
        f.get("/api/v2/metrics/refused_events").await.status,
        StatusCode::NOT_FOUND
    );

    // Patched both ways; a refused patch changes nothing.
    let moved = ok(f
        .patch(
            "/api/v2/metrics/plain_events",
            json!({"attributed_to": to_site}),
        )
        .await);
    assert_eq!(moved["attributed_to"], to_site);
    let back = ok(f
        .patch(
            "/api/v2/metrics/plain_events",
            json!({"attributed_to": {"type": "customer"}}),
        )
        .await);
    assert_eq!(back["attributed_to"], json!({"type": "customer"}));
    refused(
        &f.patch(
            "/api/v2/metrics/plain_events",
            json!({"attributed_to": {"type": "resource_type", "resource_type": "nope"}}),
        )
        .await,
        "/attributed_to/resource_type",
        "not_found",
    );
    refused(
        &f.patch(
            "/api/v2/metrics/plain_events",
            json!({"attributed_to": {"type": "resource_type", "resource_type": "legacy"}}),
        )
        .await,
        "/attributed_to",
        "invalid",
    );
    assert_eq!(
        ok(f.get("/api/v2/metrics/plain_events").await)["attributed_to"],
        json!({"type": "customer"})
    );

    // The preview: where the last 7 days' usage would go, and the drafts it recomputes; it
    // changes nothing.
    let preview = ok(f
        .post(
            "/api/v2/metrics/plain_events/attribution/preview",
            json!({"attributed_to": to_site}),
        )
        .await);
    assert!(preview["from"].is_string(), "{preview:#}");
    assert!(preview["until"].is_string());
    assert!(preview["sampled_customers"].is_u64());
    assert_eq!(preview["sampled"], false);
    for total in [
        "resource_subscriptions",
        "customer_subscriptions",
        "not_billed",
    ] {
        assert!(
            preview[total].is_string(),
            "{total}: decimals read as strings"
        );
    }
    assert_eq!(preview["draft_invoices"], 0);
    refused(
        &f.post(
            "/api/v2/metrics/plain_events/attribution/preview",
            json!({"attributed_to": {"type": "resource_type", "resource_type": "nope"}}),
        )
        .await,
        "/attributed_to/resource_type",
        "not_found",
    );
    assert_eq!(
        ok(f.get("/api/v2/metrics/plain_events").await)["attributed_to"],
        json!({"type": "customer"})
    );
}
