//! What a customer may use through API v2 (ADR-0013 §25): resource types and a customer's
//! resources, the snapshot with its ETag, the check (POST and GET, a resource by id or by type and
//! key, explain, `current_count`, the v1 inputs it refuses), usage reported by the app, the
//! customer's overrides with their history, portal-token claims; then the check's limits counted
//! from events: grace, overage, `retry_at`, `near_limit`, thresholds.
use crate::data::ids::METRIC_BANDWIDTH;
use crate::helpers::credit_recompute::{
    self, activated_at, add_entitlement, allowing, denying, linear_subscription, plain,
};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::test_rest_v2::{Answer, Fixture, fixture};
use common_domain::ids::SubscriptionId;
use meteroid_grpc::meteroid::api;
use meteroid_store::domain::entitlement_policy::{FeaturePolicy, Resets};
use reqwest::StatusCode;
use serde_json::{Value, json};

const SPOTIFY: &str = "/api/v2/customers/ext:spotify";

/// `site`, keyed by `site_id`, with an environment.
async fn sites(f: &Fixture) -> Value {
    let created = f
        .post(
            "/api/v2/resource-types",
            json!({
                "code": "site",
                "name": "Site",
                "plural_name": "Sites",
                "keys": ["site_id"],
                "attributes": {"environment": ["production", "staging"]}
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    created.body
}

/// A stored site of the customer (`/api/v2/customers/<customer>`), its id.
async fn site(f: &Fixture, customer: &str, key: &str, body: Value) -> String {
    let stored = f
        .put(&format!("{customer}/resources/site/{key}"), body)
        .await;
    assert_eq!(stored.status, StatusCode::CREATED, "{:#}", stored.body);
    stored.body["id"].as_str().unwrap().to_owned()
}

async fn feature(f: &Fixture, body: Value) -> Value {
    let created = f.post("/api/v2/features", body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    created.body
}

/// `seats` (the customer's) and `projects` (per site), limits your app counts: 10 and 3 by
/// default.
async fn app_counted(f: &Fixture) {
    feature(
        f,
        json!({
            "code": "seats",
            "name": "Seats",
            "type": {"type": "limit", "counted_by": "app"},
            "defaults": [{"feature": "seats", "value": {"type": "limit", "limit": "10"}}]
        }),
    )
    .await;
    feature(
        f,
        json!({
            "code": "projects",
            "name": "Projects",
            "type": {"type": "limit", "counted_by": "app"},
            "applies_per": "site",
            "defaults": [{"feature": "projects", "value": {"type": "limit", "limit": "3"}}]
        }),
    )
    .await;
}

fn listed(answer: &Answer) -> &Vec<Value> {
    answer.body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {:#}", answer.body))
}

fn in_snapshot<'a>(snapshot: &'a Value, code: &str) -> Option<&'a Value> {
    snapshot["entitlements"]
        .as_array()
        .unwrap_or_else(|| panic!("a snapshot: {snapshot:#}"))
        .iter()
        .find(|e| e["feature"]["code"] == code)
}

fn of_snapshot<'a>(snapshot: &'a Value, code: &str) -> &'a Value {
    in_snapshot(snapshot, code).unwrap_or_else(|| panic!("{code} in {snapshot:#}"))
}

fn limit(n: &str) -> Value {
    json!({"type": "limit", "limit": n})
}

/// Refused with `status`, the first field error at `path`.
fn refused_at(answer: &Answer, status: StatusCode, path: &str) {
    assert_eq!(answer.status, status, "{path}: {:#}", answer.body);
    assert_eq!(
        answer.header("content-type"),
        Some("application/problem+json")
    );
    assert_eq!(answer.body["errors"][0]["path"], path, "{:#}", answer.body);
}

/// Rules: ADR-0013 §25
#[tokio::test]
async fn test_rest_v2_resource_types_and_customer_resources() {
    let f = fixture().await;

    // A type: its code and keys are the tenant's alone.
    let site_type = sites(&f).await;
    assert!(site_type["id"].as_str().unwrap().starts_with("rty_"));
    assert_eq!(site_type["code"], "site");
    assert_eq!(site_type["keys"], json!(["site_id"]));
    assert_eq!(
        site_type["attributes"],
        json!({"environment": ["production", "staging"]})
    );
    assert_eq!(site_type["archived"], false);
    assert!(site_type.get("parent_type_id").is_none());
    let taken = f
        .post(
            "/api/v2/resource-types",
            json!({"code": "site", "name": "Again", "plural_name": "Agains", "keys": ["other_id"]}),
        )
        .await;
    refused_at(&taken, StatusCode::CONFLICT, "/code");
    assert_eq!(taken.body["code"], "code_taken");
    let key_taken = f
        .post(
            "/api/v2/resource-types",
            json!({"code": "workspace", "name": "Workspace", "plural_name": "Workspaces",
                   "keys": ["site_id"]}),
        )
        .await;
    refused_at(&key_taken, StatusCode::CONFLICT, "/keys");
    assert_eq!(key_taken.body["code"], "key_taken");
    let keyless = f
        .post(
            "/api/v2/resource-types",
            json!({"code": "workspace", "name": "Workspace", "plural_name": "Workspaces",
                   "keys": []}),
        )
        .await;
    refused_at(&keyless, StatusCode::UNPROCESSABLE_ENTITY, "/keys");
    let workspace = f
        .post(
            "/api/v2/resource-types",
            json!({"code": "workspace", "name": "Workspace", "plural_name": "Workspaces",
                   "keys": ["workspace_id"]}),
        )
        .await;
    assert_eq!(
        workspace.status,
        StatusCode::CREATED,
        "{:#}",
        workspace.body
    );

    // A merge patch: lists replace, a name cannot be cleared, a key stays the tenant's alone.
    let patched = f
        .patch(
            "/api/v2/resource-types/site",
            json!({"name": "Website", "attributes": {"environment": ["production", "staging", "qa"]}}),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["name"], "Website");
    assert_eq!(patched.body["plural_name"], "Sites");
    assert_eq!(
        patched.body["attributes"]["environment"],
        json!(["production", "staging", "qa"])
    );
    let by_id = f
        .get(&format!(
            "/api/v2/resource-types/{}",
            site_type["id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(by_id.status, StatusCode::OK, "{:#}", by_id.body);
    assert_eq!(by_id.body["name"], "Website");
    let clashing = f
        .patch(
            "/api/v2/resource-types/workspace",
            json!({"keys": ["site_id"]}),
        )
        .await;
    assert_eq!(clashing.status, StatusCode::CONFLICT, "{:#}", clashing.body);
    assert_eq!(clashing.body["code"], "key_taken");
    let nameless = f
        .patch("/api/v2/resource-types/site", json!({"name": null}))
        .await;
    refused_at(&nameless, StatusCode::UNPROCESSABLE_ENTITY, "/name");

    // Archived and back; the list leaves an archived type out unless asked.
    let archived = f
        .post("/api/v2/resource-types/workspace/archive", json!({}))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["archived"], true);
    assert!(archived.body["archived_at"].is_string());
    let live = f.get("/api/v2/resource-types").await;
    assert_eq!(live.status, StatusCode::OK, "{:#}", live.body);
    assert!(live.body.get("has_more").is_none(), "not paged");
    let codes = |answer: &Answer| -> Vec<String> {
        listed(answer)
            .iter()
            .map(|t| t["code"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(codes(&live).contains(&"site".to_owned()), "{:#}", live.body);
    assert!(
        !codes(&live).contains(&"workspace".to_owned()),
        "{:#}",
        live.body
    );
    let all = f.get("/api/v2/resource-types?include_archived=true").await;
    assert!(
        codes(&all).contains(&"workspace".to_owned()),
        "{:#}",
        all.body
    );

    // An archived type takes no new resource.
    let on_archived = f
        .put(&format!("{SPOTIFY}/resources/workspace/w1"), json!({}))
        .await;
    assert_eq!(
        on_archived.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        on_archived.body
    );
    assert_eq!(on_archived.body["errors"][0]["code"], "archived");
    let back = f
        .post("/api/v2/resource-types/workspace/unarchive", json!({}))
        .await;
    assert_eq!(back.body["archived"], false, "{:#}", back.body);
    assert!(back.body.get("archived_at").is_none());

    // A resource: created (201), then its name and attributes replaced whole (200).
    let created = f
        .put(
            &format!("{SPOTIFY}/resources/site/s1"),
            json!({"name": "Main", "attributes": {"environment": "production"}}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let s1 = created.body["id"].as_str().unwrap().to_owned();
    assert!(s1.starts_with("res_"), "{s1}");
    assert_eq!(created.body["type"]["code"], "site");
    assert!(
        created.body["type"]["id"]
            .as_str()
            .unwrap()
            .starts_with("rty_")
    );
    assert_eq!(created.body["key"], "s1");
    assert_eq!(created.body["name"], "Main");
    assert_eq!(
        created.body["attributes"],
        json!({"environment": "production"})
    );
    assert_eq!(created.body["archived"], false);
    let replaced = f
        .put(
            &format!("{SPOTIFY}/resources/site/s1"),
            json!({"name": "Main site"}),
        )
        .await;
    assert_eq!(replaced.status, StatusCode::OK, "{:#}", replaced.body);
    assert_eq!(replaced.body["id"], s1.as_str());
    assert_eq!(replaced.body["name"], "Main site");
    assert_eq!(replaced.body["attributes"], json!({}));
    let read = f.get(&format!("{SPOTIFY}/resources/site/s1")).await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body, replaced.body);
    let missing = f.get(&format!("{SPOTIFY}/resources/site/nope")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{:#}", missing.body);
    assert_eq!(missing.body["code"], "not_found");
    let undeclared = f
        .put(
            &format!("{SPOTIFY}/resources/site/s9"),
            json!({"attributes": {"region": "eu"}}),
        )
        .await;
    assert_eq!(
        undeclared.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        undeclared.body
    );
    assert!(
        undeclared.body["errors"][0]["path"]
            .as_str()
            .unwrap()
            .starts_with("/attributes"),
        "{:#}",
        undeclared.body
    );
    let unknown_type = f
        .put(&format!("{SPOTIFY}/resources/planet/p1"), json!({}))
        .await;
    assert_eq!(unknown_type.status, StatusCode::NOT_FOUND);

    // Listed by keyset in creation order, by type, archived ones left out unless asked.
    let s2 = site(&f, SPOTIFY, "s2", json!({})).await;
    let s3 = site(&f, SPOTIFY, "s3", json!({})).await;
    let w1 = f
        .put(&format!("{SPOTIFY}/resources/workspace/w1"), json!({}))
        .await;
    assert_eq!(w1.status, StatusCode::CREATED, "{:#}", w1.body);
    let ids = |answer: &Answer| -> Vec<String> {
        listed(answer)
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let first = f
        .get(&format!("{SPOTIFY}/resources?type=site&limit=2"))
        .await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(ids(&first), vec![s1.clone(), s2.clone()]);
    assert_eq!(first.body["has_more"], true);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f
        .get(&format!(
            "{SPOTIFY}/resources?type=site&limit=2&after={cursor}"
        ))
        .await;
    assert_eq!(ids(&second), vec![s3.clone()]);
    assert_eq!(second.body["has_more"], false);
    assert!(second.body["next_cursor"].is_null());
    let workspaces = f.get(&format!("{SPOTIFY}/resources?type=workspace")).await;
    assert_eq!(listed(&workspaces).len(), 1);
    assert_eq!(listed(&workspaces)[0]["type"]["code"], "workspace");
    let every = f.get(&format!("{SPOTIFY}/resources")).await;
    assert_eq!(listed(&every).len(), 4, "{:#}", every.body);
    let uber = f.get("/api/v2/customers/ext:uber/resources").await;
    assert_eq!(listed(&uber).len(), 0, "a resource stays with its customer");
    let no_such_type = f.get(&format!("{SPOTIFY}/resources?type=planet")).await;
    assert_eq!(no_such_type.status, StatusCode::NOT_FOUND);
    let bad_cursor = f
        .get(&format!("{SPOTIFY}/resources?after=not-a-cursor"))
        .await;
    assert_eq!(bad_cursor.status, StatusCode::UNPROCESSABLE_ENTITY);

    let archived = f
        .post(&format!("{SPOTIFY}/resources/site/s3/archive"), json!({}))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{:#}", archived.body);
    assert_eq!(archived.body["archived"], true);
    let kept = f
        .put(
            &format!("{SPOTIFY}/resources/site/s3"),
            json!({"name": "Old"}),
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{:#}", kept.body);
    assert_eq!(
        kept.body["archived"], true,
        "a PUT leaves its archive alone"
    );
    let live = f.get(&format!("{SPOTIFY}/resources?type=site")).await;
    assert_eq!(ids(&live), vec![s1.clone(), s2.clone()]);
    let with_archived = f
        .get(&format!(
            "{SPOTIFY}/resources?type=site&include_archived=true"
        ))
        .await;
    assert_eq!(ids(&with_archived), vec![s1.clone(), s2.clone(), s3]);
    let restored = f
        .post(&format!("{SPOTIFY}/resources/site/s3/unarchive"), json!({}))
        .await;
    assert_eq!(restored.body["archived"], false, "{:#}", restored.body);

    // Deleted while nothing names it; an override naming it keeps it.
    let deleted = f.delete(&format!("{SPOTIFY}/resources/site/s2")).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{:#}", deleted.body);
    assert_eq!(
        f.get(&format!("{SPOTIFY}/resources/site/s2")).await.status,
        StatusCode::NOT_FOUND
    );
    app_counted(&f).await;
    let named = f
        .post(
            &format!("{SPOTIFY}/entitlement-overrides"),
            json!({"feature": "projects", "value": limit("5"), "resource": {"id": s1}}),
        )
        .await;
    assert_eq!(named.status, StatusCode::CREATED, "{:#}", named.body);
    let referenced = f.delete(&format!("{SPOTIFY}/resources/site/s1")).await;
    assert_eq!(
        referenced.status,
        StatusCode::CONFLICT,
        "{:#}",
        referenced.body
    );
    assert_eq!(referenced.body["code"], "conflict");
    assert_eq!(
        f.get(&format!("{SPOTIFY}/resources/site/s1")).await.status,
        StatusCode::OK
    );

    // The v2 document carries them.
    let spec = f
        .send(f.client.get(f.url("/api-docs/v2/openapi.json")))
        .await;
    let paths = &spec.body["paths"];
    assert!(paths["/api/v2/resource-types"]["post"].is_object());
    assert!(paths["/api/v2/resource-types/{id_or_code}"]["patch"].is_object());
    assert!(paths["/api/v2/customers/{id_or_alias}/resources"]["get"].is_object());
    assert!(paths["/api/v2/customers/{id_or_alias}/resources/{type}/{key}"]["put"].is_object());
}

/// Rules: ADR-0013 §25
#[tokio::test]
async fn test_rest_v2_customer_snapshot_and_check() {
    let f = fixture().await;
    sites(&f).await;
    let s1 = site(
        &f,
        SPOTIFY,
        "s1",
        json!({"attributes": {"environment": "production"}}),
    )
    .await;
    let uber_site = site(&f, "/api/v2/customers/ext:uber", "u1", json!({})).await;
    app_counted(&f).await;
    feature(
        &f,
        json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"}}),
    )
    .await;

    // The snapshot: every feature reaching the customer once, its ETag the body's.
    let snapshot = f.get(&format!("{SPOTIFY}/entitlements")).await;
    assert_eq!(snapshot.status, StatusCode::OK, "{:#}", snapshot.body);
    let etag = snapshot.body["etag"].as_str().unwrap().to_owned();
    assert_eq!(
        snapshot.header("etag"),
        Some(format!("\"{etag}\"").as_str())
    );
    assert!(
        snapshot.body.get("resource").is_none(),
        "{:#}",
        snapshot.body
    );
    assert!(snapshot.body.get("has_more").is_none(), "not paged");
    assert_eq!(snapshot.body["attributes_ignored"], json!([]));
    assert!(
        in_snapshot(&snapshot.body, "sso").is_none(),
        "no row reaches it: {:#}",
        snapshot.body
    );
    let seats = of_snapshot(&snapshot.body, "seats");
    assert!(
        seats["feature"]["id"]
            .as_str()
            .unwrap()
            .starts_with("feat_")
    );
    assert_eq!(seats["type"], "limit");
    assert_eq!(seats["name"], "Seats", "{seats:#}");
    assert!(
        seats["feature"].get("name").is_none(),
        "the feature named by its ref"
    );
    assert_eq!(seats["included_by"], "customer");
    assert!(seats.get("scope").is_none(), "{seats:#}");
    assert_eq!(seats["value"], limit("10"));
    let level = &seats["levels"][0];
    assert_eq!(level["level"], "customer");
    assert!(level.get("resource").is_none(), "{level:#}");
    assert!(level.get("path").is_none(), "{level:#}");
    assert_eq!(level["primary"], true);
    assert_eq!(level["used"], "0");
    assert_eq!(level["policy"]["at_limit"], "deny_new");
    assert_eq!(level["policy"]["resets"], json!({"type": "never"}));
    assert!(
        level.get("window").is_none(),
        "counted by the app: no window"
    );
    assert_eq!(level["origin"]["owner"]["type"], "feature");
    let projects = of_snapshot(&snapshot.body, "projects");
    assert_eq!(projects["applies_per"]["code"], "site");
    assert_eq!(projects["levels"][0]["reason"], "resource_required");
    assert!(projects.get("value").is_none(), "{projects:#}");

    // Unchanged values answer 304; a new override changes the etag.
    let if_none_match = |etag: &str| {
        f.client
            .get(f.url(&format!("{SPOTIFY}/entitlements")))
            .bearer_auth(&f.api_key)
            .header("If-None-Match", format!("\"{etag}\""))
    };
    let unchanged = f.send(if_none_match(&etag)).await;
    assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
    assert_eq!(
        unchanged.header("etag"),
        Some(format!("\"{etag}\"").as_str())
    );
    let raised = f
        .post(
            &format!("{SPOTIFY}/entitlement-overrides"),
            json!({"feature": "seats", "value": limit("20")}),
        )
        .await;
    assert_eq!(raised.status, StatusCode::CREATED, "{:#}", raised.body);
    let changed = f.send(if_none_match(&etag)).await;
    assert_eq!(changed.status, StatusCode::OK, "{:#}", changed.body);
    assert_ne!(changed.body["etag"], etag.as_str());
    let seats = of_snapshot(&changed.body, "seats");
    assert_eq!(seats["value"], limit("20"));
    assert_eq!(
        seats["levels"][0]["origin"]["entitlement"],
        raised.body["id"]
    );
    assert_eq!(seats["levels"][0]["origin"]["owner"]["type"], "customer");
    assert_eq!(seats["levels"][0]["origin"]["effect"], "raise_to");

    // A resource by id or by type and key, stored or not; another customer's is refused.
    let by_key = f
        .get(&format!(
            "{SPOTIFY}/entitlements?resource[type]=site&resource[key]=s1"
        ))
        .await;
    assert_eq!(by_key.status, StatusCode::OK, "{:#}", by_key.body);
    assert_eq!(
        by_key.body["resource"],
        json!({"type": "site", "key": "s1", "id": s1})
    );
    let projects = of_snapshot(&by_key.body, "projects");
    assert_eq!(projects["value"], limit("3"));
    assert_eq!(projects["levels"][0]["level"], "resource_type");
    assert_eq!(projects["levels"][0]["resource_type"]["code"], "site");
    assert_eq!(
        projects["levels"][0]["resource"],
        json!({"type": "site", "key": "s1"})
    );
    let by_id = f
        .get(&format!("{SPOTIFY}/entitlements?resource={s1}"))
        .await;
    assert_eq!(by_id.status, StatusCode::OK, "{:#}", by_id.body);
    assert_eq!(by_id.body["resource"], by_key.body["resource"]);
    let ad_hoc = f
        .get(&format!(
            "{SPOTIFY}/entitlements?resource[type]=site&resource[key]=s9"
        ))
        .await;
    assert_eq!(ad_hoc.status, StatusCode::OK, "{:#}", ad_hoc.body);
    assert_eq!(
        ad_hoc.body["resource"],
        json!({"type": "site", "key": "s9"})
    );
    let asserted = f
        .get(&format!(
            "{SPOTIFY}/entitlements?resource={s1}&attributes[site.environment]=staging"
        ))
        .await;
    assert_eq!(asserted.status, StatusCode::OK, "{:#}", asserted.body);
    assert_eq!(
        asserted.body["attributes_ignored"],
        json!(["site.environment"]),
        "the stored resource's own win"
    );
    for query in [
        format!("resource={uber_site}"),
        "resource=not-an-id".to_owned(),
        "resource[type]=planet&resource[key]=p1".to_owned(),
    ] {
        let refused = f.get(&format!("{SPOTIFY}/entitlements?{query}")).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}: {:#}",
            refused.body
        );
        assert!(
            refused.body["errors"][0]["path"]
                .as_str()
                .unwrap()
                .starts_with("resource"),
            "{query}: {:#}",
            refused.body
        );
    }
    let half = f
        .get(&format!("{SPOTIFY}/entitlements?resource[type]=site"))
        .await;
    refused_at(&half, StatusCode::UNPROCESSABLE_ENTITY, "resource[key]");
    let v1_form = f
        .get(&format!("{SPOTIFY}/entitlements?resources[site_id]=s1"))
        .await;
    refused_at(&v1_form, StatusCode::UNPROCESSABLE_ENTITY, "resources");
    let unknown_attribute = f
        .get(&format!(
            "{SPOTIFY}/entitlements?resource={s1}&attributes[site.region]=eu"
        ))
        .await;
    assert_eq!(
        unknown_attribute.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        unknown_attribute.body
    );

    // One feature of it, by code or id, with no ETag; a feature not active is not found.
    let one = f.get(&format!("{SPOTIFY}/entitlements/seats")).await;
    assert_eq!(one.status, StatusCode::OK, "{:#}", one.body);
    assert_eq!(one.body["feature"]["code"], "seats");
    assert_eq!(one.body["value"], limit("20"));
    assert!(one.header("etag").is_none());
    let seats_id = one.body["feature"]["id"].as_str().unwrap().to_owned();
    let by_feature_id = f.get(&format!("{SPOTIFY}/entitlements/{seats_id}")).await;
    assert_eq!(by_feature_id.body["feature"]["code"], "seats");
    let on_site = f
        .get(&format!("{SPOTIFY}/entitlements/projects?resource={s1}"))
        .await;
    assert_eq!(on_site.status, StatusCode::OK, "{:#}", on_site.body);
    assert_eq!(on_site.body["levels"][0]["resource"]["key"], "s1");
    assert_eq!(
        f.get(&format!("{SPOTIFY}/entitlements/no-such-feature"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    feature(
        &f,
        json!({"code": "beta", "name": "Beta", "type": {"type": "boolean"},
               "defaults": [{"feature": "beta", "value": {"type": "on"}}]}),
    )
    .await;
    let beta = f.get(&format!("{SPOTIFY}/entitlements/beta")).await;
    assert_eq!(beta.status, StatusCode::OK, "{:#}", beta.body);
    assert_eq!(beta.body["value"], json!({"type": "on"}));
    let disabled = f.post("/api/v2/features/beta/disable", json!({})).await;
    assert_eq!(disabled.status, StatusCode::OK, "{:#}", disabled.body);
    let gone = f.get(&format!("{SPOTIFY}/entitlements/beta")).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{:#}", gone.body);

    // The check: allowed within the limit, its usage and what is left.
    let check = format!("{SPOTIFY}/entitlements/check");
    let within = f
        .post(&check, json!({"feature": "seats", "requested": "20"}))
        .await;
    assert_eq!(within.status, StatusCode::OK, "{:#}", within.body);
    let answer = &within.body;
    assert_eq!(answer["allowed"], true);
    assert!(answer.get("reason").is_none(), "{answer:#}");
    assert!(answer.get("reason_type").is_none());
    assert_eq!(answer["feature"]["code"], "seats");
    assert_eq!(answer["feature"]["id"], seats_id.as_str());
    assert_eq!(answer["value"], limit("20"));
    assert_eq!(answer["used"], "0");
    assert_eq!(answer["limit"], "20");
    assert_eq!(answer["remaining"], "20");
    assert_eq!(answer["usage_source"], "read");
    assert_eq!(answer["in_overage"], false);
    assert_eq!(answer["attributes_ignored"], json!([]));
    assert_eq!(answer["warnings"], json!([]));
    assert!(answer["as_of"].is_string());
    assert!(answer.get("retry_at").is_none());
    assert!(answer.get("hint").is_none());
    for v1 in ["has_access", "within_limit", "exact", "limits", "lag_ms"] {
        assert!(answer.get(v1).is_none(), "{v1}: {answer:#}");
    }
    let own = &answer["levels"][0];
    assert_eq!(own["level"], "customer");
    assert!(own.get("resource").is_none(), "{own:#}");
    assert_eq!(own["primary"], true);
    assert_eq!(own["allowed"], true);
    assert!(own.get("explain").is_none());
    assert!(own.get("reset_at").is_none(), "a count that never resets");

    // Past it: refused, and never lifting by itself (the count does not reset).
    let past = f
        .get(&format!("{SPOTIFY}/entitlements/seats/check?requested=21"))
        .await;
    assert_eq!(past.status, StatusCode::OK, "{:#}", past.body);
    assert_eq!(past.body["allowed"], false);
    assert_eq!(past.body["reason"], "limit_reached");
    assert_eq!(past.body["reason_type"], "denied");
    assert_eq!(past.body["levels"][0]["allowed"], false);
    assert_eq!(past.body["levels"][0]["reason"], "limit_reached");
    assert!(past.body.get("retry_at").is_none(), "{:#}", past.body);
    let by_id = f
        .get(&format!(
            "{SPOTIFY}/entitlements/{seats_id}/check?requested=1"
        ))
        .await;
    assert_eq!(by_id.body["allowed"], true, "{:#}", by_id.body);
    let nothing = f
        .get(&format!("{SPOTIFY}/entitlements/seats/check?requested=0"))
        .await;
    assert_eq!(nothing.body["allowed"], true, "{:#}", nothing.body);

    // `current_count` stands in for the reported count.
    let counted = f
        .get(&format!(
            "{SPOTIFY}/entitlements/seats/check?requested=1&current_count=19"
        ))
        .await;
    assert_eq!(counted.body["allowed"], true, "{:#}", counted.body);
    assert_eq!(counted.body["used"], "19");
    assert_eq!(counted.body["remaining"], "1");
    assert_eq!(counted.body["near_limit"], true, "within 5% of 20");
    assert_eq!(within.body["near_limit"], false);
    let counted_over = f
        .post(
            &check,
            json!({"feature": "seats", "requested": "3", "current_count": "18"}),
        )
        .await;
    assert_eq!(
        counted_over.body["allowed"], false,
        "{:#}",
        counted_over.body
    );
    assert_eq!(counted_over.body["reason"], "limit_reached");

    // `explain`: how each level's value was made.
    let explained = f
        .post(&check, json!({"feature": "seats", "explain": true}))
        .await;
    assert_eq!(explained.status, StatusCode::OK, "{:#}", explained.body);
    let explain = &explained.body["levels"][0]["explain"];
    let contributions = explain["contributions"].as_array().unwrap();
    let default = contributions
        .iter()
        .find(|c| c["tier"] == "default")
        .unwrap_or_else(|| panic!("the default: {explain:#}"));
    assert_eq!(default["owner"]["type"], "feature");
    assert_eq!(default["effect"], "set");
    assert_eq!(default["value"], limit("10"));
    assert_eq!(default["quantity"], 1);
    assert_eq!(default["policy_override"], false);
    assert!(default.get("units").is_none());
    let customer = contributions
        .iter()
        .find(|c| c["tier"] == "customer_override")
        .unwrap_or_else(|| panic!("the override: {explain:#}"));
    assert_eq!(customer["entitlement"], raised.body["id"]);
    assert_eq!(customer["effect"], "raise_to");
    assert_eq!(customer["running"], limit("20"));
    assert!(explain["candidates"].as_array().unwrap().is_empty());
    assert!(explain["app_counts"].is_array());
    let by_query = f
        .get(&format!("{SPOTIFY}/entitlements/seats/check?explain=true"))
        .await;
    assert!(by_query.body["levels"][0]["explain"]["contributions"].is_array());

    // A feature per site: the resource by id or by type and key, stored or not.
    let named_by_key = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"type": "site", "key": "s1"}}),
        )
        .await;
    assert_eq!(
        named_by_key.status,
        StatusCode::OK,
        "{:#}",
        named_by_key.body
    );
    assert_eq!(named_by_key.body["allowed"], true);
    assert_eq!(named_by_key.body["limit"], "3");
    let own = &named_by_key.body["levels"][0];
    assert_eq!(own["level"], "resource_type");
    assert_eq!(own["resource_type"]["code"], "site");
    assert_eq!(own["resource"], json!({"type": "site", "key": "s1"}));
    assert!(own.get("path").is_none(), "{own:#}");
    assert_eq!(own["primary"], true);
    let named_by_id = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"id": s1}}),
        )
        .await;
    assert_eq!(named_by_id.body["levels"][0]["resource"]["key"], "s1");
    let by_get = f
        .get(&format!(
            "{SPOTIFY}/entitlements/projects/check?resource[type]=site&resource[key]=s1&requested=3"
        ))
        .await;
    assert_eq!(by_get.body["allowed"], true, "{:#}", by_get.body);
    let by_get_id = f
        .get(&format!(
            "{SPOTIFY}/entitlements/projects/check?resource={s1}&requested=4"
        ))
        .await;
    assert_eq!(by_get_id.body["allowed"], false, "{:#}", by_get_id.body);
    let ad_hoc = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"type": "site", "key": "s9"},
                   "attributes": {"site.environment": "staging"}}),
        )
        .await;
    assert_eq!(ad_hoc.body["allowed"], true, "{:#}", ad_hoc.body);
    assert_eq!(
        ad_hoc.body["levels"][0]["resource"],
        json!({"type": "site", "key": "s9"})
    );
    let overridden = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"id": s1},
                   "attributes": {"site.environment": "staging"}}),
        )
        .await;
    assert_eq!(
        overridden.body["attributes_ignored"],
        json!(["site.environment"])
    );
    let unnamed = f.post(&check, json!({"feature": "projects"})).await;
    assert_eq!(unnamed.status, StatusCode::OK, "{:#}", unnamed.body);
    assert_eq!(unnamed.body["allowed"], false);
    assert_eq!(unnamed.body["reason"], "resource_required");
    assert_eq!(unnamed.body["reason_type"], "misconfigured");

    // Refused at the field naming it: another customer's resource, an unknown type, an
    // unknown attribute, a negative or unreadable request.
    let theirs = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"id": uber_site}}),
        )
        .await;
    refused_at(&theirs, StatusCode::UNPROCESSABLE_ENTITY, "/resource");
    assert_eq!(theirs.body["errors"][0]["code"], "not_found");
    let theirs_by_query = f
        .get(&format!(
            "{SPOTIFY}/entitlements/projects/check?resource={uber_site}"
        ))
        .await;
    refused_at(
        &theirs_by_query,
        StatusCode::UNPROCESSABLE_ENTITY,
        "resource",
    );
    let planet = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"type": "planet", "key": "p1"}}),
        )
        .await;
    refused_at(&planet, StatusCode::UNPROCESSABLE_ENTITY, "/resource/type");
    let region = f
        .post(
            &check,
            json!({"feature": "projects", "resource": {"id": s1}, "attributes": {"site.region": "eu"}}),
        )
        .await;
    assert_eq!(
        region.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        region.body
    );
    let negative = f
        .post(&check, json!({"feature": "seats", "requested": "-1"}))
        .await;
    refused_at(&negative, StatusCode::UNPROCESSABLE_ENTITY, "/requested");
    let negative_count = f
        .post(&check, json!({"feature": "seats", "current_count": "-1"}))
        .await;
    refused_at(
        &negative_count,
        StatusCode::UNPROCESSABLE_ENTITY,
        "/current_count",
    );
    for query in ["requested=lots", "requested=-1"] {
        let refused = f
            .get(&format!("{SPOTIFY}/entitlements/seats/check?{query}"))
            .await;
        refused_at(&refused, StatusCode::UNPROCESSABLE_ENTITY, "requested");
    }
    let unreadable = f
        .post(&check, json!({"feature": "seats", "requested": "lots"}))
        .await;
    assert_eq!(
        unreadable.status,
        StatusCode::BAD_REQUEST,
        "{:#}",
        unreadable.body
    );
    let unknown = f.post(&check, json!({"feature": "no-such-feature"})).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{:#}", unknown.body);
    assert_eq!(unknown.body["code"], "not_found");
    let nobody = f
        .post(
            "/api/v2/customers/ext:nobody/entitlements/check",
            json!({"feature": "seats"}),
        )
        .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);

    // v1's inputs no check takes: a 400 in the body, a 422 naming the parameter in the query.
    for body in [
        json!({"feature": "seats", "subscription_id": "sub_1"}),
        json!({"feature": "seats", "subscription": "sub_1"}),
        json!({"feature": "seats", "group": {"region": "eu"}}),
        json!({"feature": "seats", "group_by": "region"}),
        json!({"feature": "seats", "as_of": "2026-01-01T00:00:00Z"}),
        json!({"feature": "projects", "resources": {"site_id": "s1"}}),
    ] {
        let refused = f.post(&check, body.clone()).await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{body}: {:#}",
            refused.body
        );
        assert_eq!(refused.body["code"], "invalid_request");
    }
    for (query, parameter) in [
        ("group[region]=eu", "group"),
        ("group_by=region", "group_by"),
        ("as_of=2026-01-01T00:00:00Z", "as_of"),
        ("subscription=sub_1", "subscription"),
        ("subscription_id=sub_1", "subscription_id"),
        ("resources[site_id]=s1", "resources"),
    ] {
        let refused = f
            .get(&format!("{SPOTIFY}/entitlements/seats/check?{query}"))
            .await;
        refused_at(&refused, StatusCode::UNPROCESSABLE_ENTITY, parameter);
    }

    // A check is a read: a key sent twice with two bodies is never a reused key.
    for requested in ["1", "2"] {
        let keyed = f
            .post_once(
                &check,
                "check-key",
                json!({"feature": "seats", "requested": requested}),
            )
            .await;
        assert_eq!(keyed.status, StatusCode::OK, "{:#}", keyed.body);
        assert!(keyed.header("Idempotent-Replayed").is_none());
    }

    // The v2 document carries both forms, core's.
    let spec = f
        .send(f.client.get(f.url("/api-docs/v2/openapi.json")))
        .await;
    let paths = &spec.body["paths"];
    assert!(paths["/api/v2/customers/{id_or_alias}/entitlements/check"]["post"].is_object());
    assert!(
        paths["/api/v2/customers/{id_or_alias}/entitlements/{feature}/check"]["get"].is_object()
    );
    assert!(paths["/api/v2/customers/{id_or_alias}/entitlements/{feature}"]["get"].is_object());
}

/// Rules: ADR-0013 §25
#[tokio::test]
async fn test_rest_v2_usage_reported_by_the_app() {
    let f = fixture().await;
    sites(&f).await;
    let s1 = site(&f, SPOTIFY, "s1", json!({})).await;
    app_counted(&f).await;
    feature(
        &f,
        json!({"code": "sso", "name": "SSO", "type": {"type": "boolean"}}),
    )
    .await;
    let seats = format!("{SPOTIFY}/entitlements/seats/usage");

    // Reported under an Idempotency-Key, the store's operation key.
    let keyless = f.post(&seats, json!({"set": "3"})).await;
    refused_at(&keyless, StatusCode::BAD_REQUEST, "Idempotency-Key");
    assert_eq!(keyless.body["code"], "idempotency_key_required");
    assert_eq!(keyless.body["errors"][0]["code"], "required");
    let set = f.post_once(&seats, "seats-1", json!({"set": "3"})).await;
    assert_eq!(set.status, StatusCode::OK, "{:#}", set.body);
    assert_eq!(set.body["feature"]["code"], "seats");
    assert!(
        set.body["feature"]["id"]
            .as_str()
            .unwrap()
            .starts_with("feat_")
    );
    assert!(set.body.get("resource").is_none(), "{:#}", set.body);
    assert!(set.body.get("path").is_none(), "{:#}", set.body);
    assert_eq!(
        (
            &set.body["count"],
            &set.body["applied"],
            &set.body["replayed"]
        ),
        (&json!("3"), &json!(true), &json!(false))
    );
    let again = f.post_once(&seats, "seats-1", json!({"set": "3"})).await;
    assert_eq!(again.status, StatusCode::OK, "{:#}", again.body);
    assert_eq!(again.body["count"], "3", "a retry counts once");
    let reused = f.post_once(&seats, "seats-1", json!({"set": "4"})).await;
    assert_eq!(
        reused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        reused.body
    );
    assert_eq!(reused.body["code"], "idempotency_key_reused");
    let added = f.post_once(&seats, "seats-2", json!({"add": "2"})).await;
    assert_eq!(added.body["count"], "5", "{:#}", added.body);
    let removed = f.post_once(&seats, "seats-3", json!({"add": "-1"})).await;
    assert_eq!(removed.body["count"], "4", "{:#}", removed.body);

    // An add older than the last set is recorded and changes nothing.
    let earlier = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    let late = f
        .post_once(&seats, "seats-4", json!({"add": "7", "at": earlier}))
        .await;
    assert_eq!(late.status, StatusCode::OK, "{:#}", late.body);
    assert_eq!(late.body["applied"], false);
    assert_eq!(late.body["count"], "4");
    // A set older than the operations kept could miss adds: refused.
    let long_ago = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let stale = f
        .post_once(&seats, "seats-5", json!({"set": "1", "at": long_ago}))
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{:#}", stale.body);
    assert_eq!(stale.body["code"], "stale_set");

    // Refused: none or both of set and add, a negative set, a feature your app does not count.
    for (i, body) in [
        json!({}),
        json!({"set": "1", "add": "1"}),
        json!({"set": "-1"}),
    ]
    .into_iter()
    .enumerate()
    {
        let refused = f.post_once(&seats, &format!("seats-bad-{i}"), body).await;
        refused_at(&refused, StatusCode::UNPROCESSABLE_ENTITY, "/set");
    }
    let not_counted = f
        .post_once(
            &format!("{SPOTIFY}/entitlements/sso/usage"),
            "sso-1",
            json!({"set": "1"}),
        )
        .await;
    refused_at(&not_counted, StatusCode::UNPROCESSABLE_ENTITY, "feature");
    let unknown = f
        .post_once(
            &format!("{SPOTIFY}/entitlements/no-such-feature/usage"),
            "unknown-1",
            json!({"set": "1"}),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{:#}", unknown.body);
    let customer_wide = f
        .post_once(
            &seats,
            "seats-6",
            json!({"set": "1", "resource": {"type": "site", "key": "s1"}}),
        )
        .await;
    refused_at(
        &customer_wide,
        StatusCode::UNPROCESSABLE_ENTITY,
        "/resource",
    );

    // A feature per site counts per resource, which the report names.
    let projects = format!("{SPOTIFY}/entitlements/projects/usage");
    let unnamed = f
        .post_once(&projects, "projects-1", json!({"set": "1"}))
        .await;
    assert_eq!(
        unnamed.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{:#}",
        unnamed.body
    );
    assert_eq!(unnamed.body["code"], "resource_required");
    let by_key = f
        .post_once(
            &projects,
            "projects-2",
            json!({"set": "2", "resource": {"type": "site", "key": "s1"}}),
        )
        .await;
    assert_eq!(by_key.status, StatusCode::OK, "{:#}", by_key.body);
    assert_eq!(
        by_key.body["resource"],
        json!({"type": "site", "key": "s1"})
    );
    assert_eq!(by_key.body["count"], "2");
    let by_id = f
        .post_once(
            &projects,
            "projects-3",
            json!({"add": "1", "resource": {"id": s1}}),
        )
        .await;
    assert_eq!(by_id.body["resource"]["key"], "s1", "{:#}", by_id.body);
    assert_eq!(by_id.body["count"], "3");
    let ad_hoc = f
        .post_once(
            &projects,
            "projects-4",
            json!({"add": "1", "resource": {"type": "site", "key": "s7"}}),
        )
        .await;
    assert_eq!(
        ad_hoc.body["resource"],
        json!({"type": "site", "key": "s7"}),
        "{:#}",
        ad_hoc.body
    );

    // What is reported is what the snapshot and the check count.
    let snapshot = f
        .get(&format!("{SPOTIFY}/entitlements?resource={s1}"))
        .await;
    assert_eq!(snapshot.status, StatusCode::OK, "{:#}", snapshot.body);
    assert_eq!(
        of_snapshot(&snapshot.body, "seats")["levels"][0]["used"],
        "4"
    );
    assert_eq!(
        of_snapshot(&snapshot.body, "projects")["levels"][0]["used"],
        "3"
    );
    let full = f
        .get(&format!(
            "{SPOTIFY}/entitlements/projects/check?resource={s1}"
        ))
        .await;
    assert_eq!(full.body["allowed"], false, "3 of 3: {:#}", full.body);
    assert_eq!(full.body["reason"], "limit_reached");
    assert_eq!(full.body["used"], "3");
    assert_eq!(full.body["remaining"], "0");
    let explained = f
        .post(
            &format!("{SPOTIFY}/entitlements/check"),
            json!({"feature": "seats", "explain": true}),
        )
        .await;
    let counts = &explained.body["levels"][0]["explain"]["app_counts"];
    assert!(counts[0].get("resource").is_none(), "{counts:#}");
    assert_eq!(counts[0]["count"], "4");
    assert_eq!(counts[0]["last_set"]["count"], "3");
    assert_eq!(counts[0]["last_set"]["key"], "seats-1");
}

/// Rules: ADR-0013 §25
#[tokio::test]
async fn test_rest_v2_customer_entitlement_overrides() {
    let f = fixture().await;
    sites(&f).await;
    let s1 = site(&f, SPOTIFY, "s1", json!({})).await;
    let s2 = site(&f, SPOTIFY, "s2", json!({})).await;
    let uber_site = site(&f, "/api/v2/customers/ext:uber", "u1", json!({})).await;
    app_counted(&f).await;
    let overrides = format!("{SPOTIFY}/entitlement-overrides");

    // On the customer: `raise_to` by default.
    let created = f
        .post(
            &overrides,
            json!({"feature": "seats", "value": limit("20")}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{:#}", created.body);
    let row = &created.body;
    let id = row["id"].as_str().unwrap().to_owned();
    assert!(id.starts_with("ent_"), "{id}");
    assert_eq!(row["feature"], "seats");
    assert_eq!(row["value"], limit("20"));
    assert_eq!(row["effect"], "raise_to");
    assert_eq!(
        row["level"], "customer",
        "seats applies per customer: {row:#}"
    );
    assert!(row.get("resource_type").is_none(), "{row:#}");
    assert_eq!(row["owner"]["type"], "customer");
    assert!(row["owner"]["id"].as_str().unwrap().starts_with("cus_"));
    assert!(row.get("resource").is_none());
    assert!(row.get("revoked_at").is_none());
    assert!(row["created_at"].is_string() && row["updated_at"].is_string());

    // Another open row of its key is refused, at its feature.
    let duplicate = f
        .post(
            &overrides,
            json!({"feature": "seats", "value": limit("30"), "effect": "set"}),
        )
        .await;
    refused_at(&duplicate, StatusCode::CONFLICT, "/feature");
    assert_eq!(duplicate.body["code"], "already_exists");
    let mistyped = f
        .post(
            &overrides,
            json!({"feature": "seats", "value": {"type": "on"}}),
        )
        .await;
    refused_at(&mistyped, StatusCode::UNPROCESSABLE_ENTITY, "/value");
    let added = f
        .post(
            &overrides,
            json!({"feature": "seats", "value": limit("1"), "effect": "add"}),
        )
        .await;
    refused_at(&added, StatusCode::UNPROCESSABLE_ENTITY, "/effect");
    let unknown = f
        .post(&overrides, json!({"feature": "nope", "value": limit("1")}))
        .await;
    refused_at(&unknown, StatusCode::UNPROCESSABLE_ENTITY, "/feature");

    // On one of its stored resources, named by id or by type and key; read as `{id, type, key}`.
    let on_s1 = f
        .post(
            &overrides,
            json!({"feature": "projects", "value": limit("5"),
                   "resource": {"type": "site", "key": "s1"}}),
        )
        .await;
    assert_eq!(on_s1.status, StatusCode::CREATED, "{:#}", on_s1.body);
    assert_eq!(
        on_s1.body["resource"],
        json!({"id": s1, "type": "site", "key": "s1"})
    );
    // Written without a level, read at the feature's own: per site.
    assert_eq!(on_s1.body["level"], "resource_type", "{:#}", on_s1.body);
    assert_eq!(on_s1.body["resource_type"], "site");
    let old_spelling = f
        .post(
            &overrides,
            json!({"feature": "projects", "value": limit("5"), "level": "feature"}),
        )
        .await;
    assert_eq!(
        old_spelling.status,
        StatusCode::BAD_REQUEST,
        "{:#}",
        old_spelling.body
    );
    assert_eq!(old_spelling.body["errors"][0]["path"], "/level");
    let on_s2 = f
        .post(
            &overrides,
            json!({"feature": "projects", "value": limit("7"), "resource": {"id": s2}}),
        )
        .await;
    assert_eq!(on_s2.status, StatusCode::CREATED, "{:#}", on_s2.body);
    assert_eq!(on_s2.body["resource"]["key"], "s2");
    for resource in [
        json!({"type": "site", "key": "not-stored"}),
        json!({"id": uber_site}),
        json!({"id": "not-an-id"}),
        json!({"id": s2, "type": "site", "key": "s2"}),
        json!({}),
    ] {
        let refused = f
            .post(
                &overrides,
                json!({"feature": "projects", "value": limit("1"), "resource": resource}),
            )
            .await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{resource}: {:#}",
            refused.body
        );
        assert_eq!(refused.body["errors"][0]["path"], "/resource");
    }
    // A resource is an object: a bare id does not read.
    let bare = f
        .post(
            &overrides,
            json!({"feature": "projects", "value": limit("1"), "resource": s2}),
        )
        .await;
    assert_eq!(bare.status, StatusCode::BAD_REQUEST, "{:#}", bare.body);
    assert_eq!(
        bare.body["errors"][0]["path"], "/resource",
        "{:#}",
        bare.body
    );
    let half = f
        .post(
            &overrides,
            json!({"feature": "projects", "value": limit("1"), "resource": {"key": "s2"}}),
        )
        .await;
    refused_at(&half, StatusCode::UNPROCESSABLE_ENTITY, "/resource/type");
    let snapshot = f
        .get(&format!("{SPOTIFY}/entitlements?resource={s1}"))
        .await;
    assert_eq!(
        of_snapshot(&snapshot.body, "projects")["value"],
        limit("5"),
        "{:#}",
        snapshot.body
    );

    // Listed in creation order by keyset, by feature.
    let ids = |answer: &Answer| -> Vec<String> {
        listed(answer)
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_owned())
            .collect()
    };
    let on_s1_id = on_s1.body["id"].as_str().unwrap().to_owned();
    let on_s2_id = on_s2.body["id"].as_str().unwrap().to_owned();
    let first = f.get(&format!("{overrides}?limit=2")).await;
    assert_eq!(first.status, StatusCode::OK, "{:#}", first.body);
    assert_eq!(ids(&first), vec![id.clone(), on_s1_id.clone()]);
    assert_eq!(first.body["has_more"], true);
    let cursor = first.body["next_cursor"].as_str().unwrap().to_owned();
    let second = f.get(&format!("{overrides}?limit=2&after={cursor}")).await;
    assert_eq!(ids(&second), vec![on_s2_id.clone()]);
    assert_eq!(second.body["has_more"], false);
    let of_projects = f.get(&format!("{overrides}?feature=projects")).await;
    assert_eq!(ids(&of_projects), vec![on_s1_id.clone(), on_s2_id.clone()]);
    let none_for_uber = f
        .get("/api/v2/customers/ext:uber/entitlement-overrides")
        .await;
    assert!(listed(&none_for_uber).is_empty());
    let no_feature = f.get(&format!("{overrides}?feature=nope")).await;
    assert_eq!(no_feature.status, StatusCode::NOT_FOUND);

    // Read by id under its customer only.
    let read = f.get(&format!("{overrides}/{id}")).await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(read.body["id"], id.as_str());
    let elsewhere = f
        .get(&format!(
            "/api/v2/customers/ext:uber/entitlement-overrides/{id}"
        ))
        .await;
    assert_eq!(elsewhere.status, StatusCode::NOT_FOUND);

    // A patch writes its successor under the same id, the version before kept in its history.
    let patched = f
        .patch(
            &format!("{overrides}/{id}"),
            json!({"value": limit("25"), "reason": "Deal"}),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{:#}", patched.body);
    assert_eq!(patched.body["id"], id.as_str());
    assert_eq!(patched.body["value"], limit("25"));
    assert_eq!(patched.body["reason"], "Deal");
    assert_eq!(patched.body["effect"], "raise_to", "left out, unchanged");
    assert_eq!(patched.body["created_at"], row["created_at"]);
    let history = f.get(&format!("/api/v2/entitlements/{id}/history")).await;
    assert_eq!(history.status, StatusCode::OK, "{:#}", history.body);
    let versions = listed(&history);
    assert_eq!(versions.len(), 2, "{:#}", history.body);
    assert_eq!(versions[0]["value"], limit("25"), "newest first");
    assert!(versions[0].get("revoked_at").is_none());
    assert_eq!(versions[1]["value"], limit("20"));
    assert!(versions[1]["revoked_at"].is_string());
    let newest = f
        .get(&format!("/api/v2/entitlements/{id}/history?limit=1"))
        .await;
    assert_eq!(listed(&newest).len(), 1);
    assert_eq!(newest.body["has_more"], true);
    let cursor = newest.body["next_cursor"].as_str().unwrap().to_owned();
    let older = f
        .get(&format!(
            "/api/v2/entitlements/{id}/history?limit=1&after={cursor}"
        ))
        .await;
    assert_eq!(listed(&older)[0]["value"], limit("20"), "{:#}", older.body);
    assert_eq!(older.body["has_more"], false);
    let any_owner = f.get(&format!("/api/v2/entitlements/{id}")).await;
    assert_eq!(any_owner.status, StatusCode::OK, "{:#}", any_owner.body);
    assert_eq!(any_owner.body["value"], limit("25"));
    let cleared = f
        .patch(&format!("{overrides}/{id}"), json!({"reason": null}))
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{:#}", cleared.body);
    assert!(cleared.body.get("reason").is_none());
    for (body, path) in [
        (json!({"value": null}), "/value"),
        (json!({"effect": null}), "/effect"),
        (json!({"effect": "add"}), "/effect"),
        (json!({"value": {"type": "on"}}), "/value"),
        (
            json!({"policy": {"resets": {"type": "never"}, "at_limit": "deny_new"}}),
            "/policy",
        ),
        (
            json!({"valid_from": "2030-01-02T00:00:00Z", "valid_until": "2030-01-01T00:00:00Z"}),
            "/valid_until",
        ),
    ] {
        let refused = f.patch(&format!("{overrides}/{id}"), body.clone()).await;
        refused_at(&refused, StatusCode::UNPROCESSABLE_ENTITY, path);
    }
    let fixed = f
        .patch(&format!("{overrides}/{id}"), json!({"feature": "projects"}))
        .await;
    assert_eq!(
        fixed.status,
        StatusCode::BAD_REQUEST,
        "the feature is fixed"
    );

    // Revoked: 204, then 204 again; its history still reads.
    let revoked = f.delete(&format!("{overrides}/{id}")).await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT, "{:#}", revoked.body);
    let twice = f.delete(&format!("{overrides}/{id}")).await;
    assert_eq!(twice.status, StatusCode::NO_CONTENT, "{:#}", twice.body);
    assert_eq!(
        f.get(&format!("{overrides}/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.get(&format!("/api/v2/entitlements/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let after = f.get(&format!("/api/v2/entitlements/{id}/history")).await;
    assert_eq!(after.status, StatusCode::OK, "{:#}", after.body);
    assert!(
        listed(&after)[0]["revoked_at"].is_string(),
        "{:#}",
        after.body
    );
    let not_theirs = f
        .delete(&format!(
            "/api/v2/customers/ext:uber/entitlement-overrides/{on_s1_id}"
        ))
        .await;
    assert_eq!(not_theirs.status, StatusCode::NOT_FOUND);
    assert_eq!(ids(&f.get(&overrides).await), vec![on_s1_id, on_s2_id]);
    let snapshot = f.get(&format!("{SPOTIFY}/entitlements")).await;
    assert_eq!(
        of_snapshot(&snapshot.body, "seats")["value"],
        limit("10"),
        "back to the default"
    );
}

/// Rules: ADR-0013 §11, §25
#[tokio::test]
async fn test_rest_v2_portal_token_claims() {
    let f = fixture().await;
    sites(&f).await;
    let s1 = site(&f, SPOTIFY, "s1", json!({})).await;
    let uber_site = site(&f, "/api/v2/customers/ext:uber", "u1", json!({})).await;
    let token = format!("{SPOTIFY}/portal-token");

    let claimed = f
        .post(
            &token,
            json!({"resource": {"type": "site", "key": "s1"},
                   "attributes": {"site.environment": "staging"}}),
        )
        .await;
    assert_eq!(claimed.status, StatusCode::CREATED, "{:#}", claimed.body);
    let value = claimed.body["token"].as_str().unwrap().to_owned();
    let by_id = f.post(&token, json!({"resource": {"id": s1}})).await;
    assert_eq!(by_id.status, StatusCode::CREATED, "{:#}", by_id.body);
    let ad_hoc = f
        .post(&token, json!({"resource": {"type": "site", "key": "s9"}}))
        .await;
    assert_eq!(ad_hoc.status, StatusCode::CREATED, "{:#}", ad_hoc.body);

    // The claims are what a browser holding the token reads.
    let read = f
        .send(
            f.client
                .get(f.url("/api/client/v1/entitlements"))
                .bearer_auth(&value),
        )
        .await;
    assert_eq!(read.status, StatusCode::OK, "{:#}", read.body);
    assert_eq!(
        read.body["resource"],
        json!({"type": "site", "key": "s1", "id": s1})
    );
    let unclaimed = f
        .send(
            f.client
                .get(f.url("/api/client/v1/entitlements?resource[type]=site&resource[key]=s2"))
                .bearer_auth(&value),
        )
        .await;
    assert_eq!(
        unclaimed.status,
        StatusCode::FORBIDDEN,
        "{:#}",
        unclaimed.body
    );

    // Refused: another customer's resource, an undeclared attribute or value, attributes alone.
    let theirs = f.post(&token, json!({"resource": {"id": uber_site}})).await;
    refused_at(&theirs, StatusCode::UNPROCESSABLE_ENTITY, "/resource");
    for attributes in [
        json!({"site.region": "eu"}),
        json!({"site.environment": "moon"}),
    ] {
        let refused = f
            .post(
                &token,
                json!({"resource": {"id": s1}, "attributes": attributes.clone()}),
            )
            .await;
        refused_at(&refused, StatusCode::UNPROCESSABLE_ENTITY, "/attributes");
    }
    let without_resource = f
        .post(
            &token,
            json!({"attributes": {"site.environment": "staging"}}),
        )
        .await;
    refused_at(
        &without_resource,
        StatusCode::UNPROCESSABLE_ENTITY,
        "/attributes",
    );
    let planet = f
        .post(&token, json!({"resource": {"type": "planet", "key": "p1"}}))
        .await;
    refused_at(&planet, StatusCode::UNPROCESSABLE_ENTITY, "/resource/type");
}

// ── Limits counted from events: grace, overage, retry_at, near_limit ──

/// The metered fixture, its events read by the check, with a v2 key.
struct Metered {
    m: credit_recompute::Fixture,
    client: reqwest::Client,
    api_key: String,
}

impl Metered {
    async fn new() -> Self {
        let m = credit_recompute::fixture_over_events().await;
        m.events.events.lock().unwrap().clear();
        let auth = meteroid_it::svc_auth::login(m.setup.channel.clone()).await;
        let api_key = AllClients::from_channel(
            m.setup.channel.clone(),
            auth.token.as_str(),
            "TESTORG",
            "testslug",
        )
        .api_tokens
        .create_api_token(api::apitokens::v1::CreateApiTokenRequest {
            name: "checks-v2".to_owned(),
        })
        .await
        .unwrap()
        .into_inner()
        .api_key;
        Self {
            m,
            client: http_client_tracing::client(),
            api_key,
        }
    }

    async fn check(&self, feature: &str, query: &str) -> Answer {
        let url = format!(
            "{}/api/v2/customers/ext:uber/entitlements/{feature}/check{query}",
            self.m.setup.config.rest_api_external_url
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap();
        let answer = Answer {
            status,
            headers,
            body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
        };
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{feature}{query}: {:#}",
            answer.body
        );
        answer
    }

    async fn snapshot(&self) -> Value {
        let url = format!(
            "{}/api/v2/customers/ext:uber/entitlements",
            self.m.setup.config.rest_api_external_url
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }

    /// Uber's metered subscription, which used `total` of the bandwidth since it started.
    async fn consumed(&self, total: i64) -> SubscriptionId {
        let subscription_id = linear_subscription(&self.m).await;
        let activated = activated_at(&self.m, subscription_id).await;
        self.m.events.add(
            METRIC_BANDWIDTH,
            activated + chrono::Duration::seconds(1),
            plain(total),
        );
        subscription_id
    }

    async fn limit(&self, code: &str, limit: i64, policy: FeaturePolicy) {
        add_entitlement(&self.m, code, limit, policy).await;
    }
}

/// Rules: ADR-0013 §25, ADR-0011 §3.14
#[tokio::test]
async fn test_rest_v2_check_limits_counted_from_events() {
    let env = Metered::new().await;
    let subscription = env.consumed(40).await;
    env.limit("api-calls", 100, FeaturePolicy::default()).await;
    env.limit("near", 42, FeaturePolicy::default()).await;
    env.limit("graced", 38, denying(10)).await;
    env.limit("billed", 30, allowing()).await;
    env.limit(
        "lifetime",
        40,
        FeaturePolicy {
            resets: Resets::Never,
            ..FeaturePolicy::default()
        },
    )
    .await;
    env.limit(
        "alerted",
        1_000,
        FeaturePolicy {
            thresholds: vec![50, 80],
            ..FeaturePolicy::default()
        },
    )
    .await;

    // Read at check time from the deciding subscription's cycle; up to the limit inclusive.
    let within = env.check("api-calls", "?requested=60").await;
    let answer = &within.body;
    assert_eq!(answer["allowed"], true, "{answer:#}");
    assert_eq!(answer["used"], "40", "{answer:#}");
    assert_eq!(answer["limit"], "100");
    assert_eq!(answer["remaining"], "60");
    assert_eq!(answer["usage_source"], "read");
    assert_eq!(answer["near_limit"], false);
    assert_eq!(answer["in_overage"], false);
    let own = &answer["levels"][0];
    assert_eq!(own["level"], "customer");
    assert_eq!(
        own["deciding_subscription"]["id"],
        subscription.to_string().as_str()
    );
    assert_eq!(own["used"], "40");
    let reset_at = own["reset_at"]
        .as_str()
        .unwrap_or_else(|| panic!("a billing cycle ends: {answer:#}"))
        .to_owned();
    assert!(answer.get("retry_at").is_none(), "allowed: {answer:#}");

    // Past it: refused until the cycle ends, `retry_at` the level's reset.
    let past = env.check("api-calls", "?requested=60.5").await;
    assert_eq!(past.body["allowed"], false, "{:#}", past.body);
    assert_eq!(past.body["reason"], "limit_reached");
    assert_eq!(past.body["reason_type"], "denied");
    assert_eq!(past.body["remaining"], "60");
    assert_eq!(past.body["retry_at"], reset_at.as_str());
    assert_eq!(past.body["levels"][0]["reason"], "limit_reached");

    // Within 5% of a hard limit: allowed, and not to be cached.
    let near = env.check("near", "").await;
    assert_eq!(near.body["allowed"], true, "{:#}", near.body);
    assert_eq!(near.body["near_limit"], true);
    assert_eq!(near.body["levels"][0]["near_limit"], true);
    assert_eq!(near.body["remaining"], "2");
    let near_past = env.check("near", "?requested=3").await;
    assert_eq!(near_past.body["allowed"], false, "{:#}", near_past.body);

    // Grace: refused only past the limit plus its grace; `remaining` leaves the grace out.
    let in_grace = env.check("graced", "?requested=1.8").await;
    assert_eq!(in_grace.body["allowed"], true, "{:#}", in_grace.body);
    assert_eq!(in_grace.body["limit"], "38");
    assert_eq!(in_grace.body["used"], "40");
    assert_eq!(in_grace.body["remaining"], "0");
    assert_eq!(in_grace.body["near_limit"], true);
    let past_grace = env.check("graced", "?requested=1.9").await;
    assert_eq!(past_grace.body["allowed"], false, "{:#}", past_grace.body);
    assert_eq!(past_grace.body["reason"], "limit_reached");

    // Allowed and billed: never refused, the overage told.
    let billed = env.check("billed", "?requested=100").await;
    assert_eq!(billed.body["allowed"], true, "{:#}", billed.body);
    assert_eq!(billed.body["in_overage"], true);
    assert_eq!(billed.body["levels"][0]["in_overage"], true);
    assert_eq!(billed.body["near_limit"], false);
    assert_eq!(billed.body["remaining"], "0");
    assert!(billed.body.get("retry_at").is_none());

    // A limit that never resets: refused without a time it lifts.
    let lifetime = env.check("lifetime", "").await;
    assert_eq!(lifetime.body["allowed"], false, "{:#}", lifetime.body);
    assert_eq!(lifetime.body["reason"], "limit_reached");
    assert!(
        lifetime.body.get("retry_at").is_none(),
        "{:#}",
        lifetime.body
    );
    assert!(lifetime.body["levels"][0].get("reset_at").is_none());

    // The snapshot reads each level's policy, its thresholds, and its window.
    let snapshot = env.snapshot().await;
    let alerted = of_snapshot(&snapshot, "alerted");
    let level = &alerted["levels"][0];
    assert_eq!(level["policy"]["thresholds"], json!([50, 80]), "{level:#}");
    assert_eq!(level["policy"]["at_limit"], "deny");
    assert_eq!(level["policy"]["resets"], json!({"type": "billing_cycle"}));
    assert_eq!(level["used"], "40");
    assert!(level["window"]["start"].is_string(), "{level:#}");
    assert_eq!(
        level["deciding_subscription"]["id"],
        subscription.to_string().as_str()
    );
    let graced = of_snapshot(&snapshot, "graced");
    assert_eq!(graced["levels"][0]["policy"]["grace_pct"], 10);
    assert_eq!(
        of_snapshot(&snapshot, "billed")["levels"][0]["policy"]["at_limit"],
        "allow_and_bill"
    );
}
