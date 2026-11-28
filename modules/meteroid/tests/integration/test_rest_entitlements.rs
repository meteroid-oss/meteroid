//! REST write surface for features and entitlements: create, batch-create, update, delete,
//! archive, read back, and the tenant-ownership checks on every entity and reference a write
//! names.

use serde_json::{Value, json};

use crate::data::ids::{
    METRIC_BANDWIDTH, PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_2_LEETCODE_ID,
    PLAN_VERSION_PRO_DRAFT_ID, PRICE_LEETCODE_RATE_ID, PRODUCT_PLATFORM_FEE_ID, TENANT_ID,
};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use common_domain::ids::{BaseId, BillableMetricId, EntitlementId};
use meteroid_grpc::meteroid::api;
use rust_decimal_macros::dec;

async fn json_or_panic(res: reqwest::Response, expected: reqwest::StatusCode) -> Value {
    let status = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(status, expected, "{status}: {body}");
    serde_json::from_str(&body).unwrap()
}

async fn create_api_key(clients: &AllClients, name: &str) -> String {
    clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            api::apitokens::v1::CreateApiTokenRequest {
                name: name.to_string(),
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key
}

fn find_resolved<'a>(listed: &'a Value, code: &str) -> Option<&'a Value> {
    listed["data"]
        .as_array()
        .expect("the list response carries a data array")
        .iter()
        .find(|entry| entry["feature"]["code"] == code)
}

fn resolved_by_code<'a>(listed: &'a Value, code: &str) -> &'a Value {
    find_resolved(listed, code)
        .unwrap_or_else(|| panic!("no resolved entitlement for feature {code} in {listed}"))
}

struct RestFixture {
    setup: MeteroidSetup,
    clients: AllClients,
    auth_token: String,
    client: reqwest::Client,
    base: String,
    api_key: String,
}

impl RestFixture {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client.get(self.url(path)).bearer_auth(&self.api_key)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.client.post(self.url(path)).bearer_auth(&self.api_key)
    }

    fn patch(&self, path: &str) -> reqwest::RequestBuilder {
        self.client.patch(self.url(path)).bearer_auth(&self.api_key)
    }

    fn put(&self, path: &str) -> reqwest::RequestBuilder {
        self.client.put(self.url(path)).bearer_auth(&self.api_key)
    }

    fn delete(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .delete(self.url(path))
            .bearer_auth(&self.api_key)
    }

    async fn create_feature(&self, body: Value) -> Value {
        json_or_panic(
            self.post("/api/v1/features")
                .json(&body)
                .send()
                .await
                .unwrap(),
            reqwest::StatusCode::CREATED,
        )
        .await
    }
}

async fn start_rest_fixture(key_name: &str) -> RestFixture {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = create_api_key(&clients, key_name).await;
    let base = setup.config.rest_api_external_url.clone();

    RestFixture {
        setup,
        clients,
        auth_token: auth.token,
        client: http_client_tracing::client(),
        base,
        api_key,
    }
}

/// A second tenant in the same organization, with its own API key. The environment is the
/// caller's choice because a tenant's slug is derived from it, and the slug cache is
/// process-global: two tests sharing one would resolve to each other's tenant.
async fn second_tenant_key(
    fixture: &RestFixture,
    environment: api::tenants::v1::TenantEnvironmentEnum,
    tenant_name: &str,
    key_name: &str,
) -> String {
    let tenant = fixture
        .clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: tenant_name.to_string(),
            environment: environment as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
            copy_from: None,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    let clients = AllClients::from_channel(
        fixture.setup.channel.clone(),
        fixture.auth_token.as_str(),
        "TESTORG",
        tenant.slug.as_str(),
    );
    create_api_key(&clients, key_name).await
}

async fn status_of(request: reqwest::RequestBuilder) -> reqwest::StatusCode {
    request.send().await.unwrap().status()
}

#[tokio::test]
async fn test_rest_feature_and_entitlement_writes() {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let api_key = create_api_key(&clients, "rest-entitlement-writes").await;

    let base = setup.config.rest_api_external_url.clone();
    let client = http_client_tracing::client();

    let support = json_or_panic(
        client
            .post(format!("{base}/api/v1/features"))
            .bearer_auth(&api_key)
            .json(&json!({
                "name": "Priority support",
                "code": "priority-support",
                "description": "24/7 support",
                "kind": "BOOLEAN",
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(support["code"], "priority-support");
    assert_eq!(support["name"], "Priority support");
    assert_eq!(support["description"], "24/7 support");
    assert_eq!(support["status"], "ACTIVE");
    assert_eq!(support["feature_type"]["type"], "BOOLEAN");
    let support_id = support["id"].as_str().unwrap().to_string();

    let bandwidth = json_or_panic(
        client
            .post(format!("{base}/api/v1/features"))
            .bearer_auth(&api_key)
            .json(&json!({
                "name": "Bandwidth",
                "code": "bandwidth",
                "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": METRIC_BANDWIDTH.as_base62() },
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(
        bandwidth["feature_type"]["metric_id"],
        METRIC_BANDWIDTH.as_base62()
    );
    let bandwidth_id = bandwidth["id"].as_str().unwrap().to_string();

    // The feature-code rule is enforced on the REST payload, not only over gRPC.
    let rejected = client
        .post(format!("{base}/api/v1/features"))
        .bearer_auth(&api_key)
        .json(&json!({
            "name": "Bad code",
            "code": "Not A Code",
            "kind": "BOOLEAN",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);

    let created_feature = json_or_panic(
        client
            .get(format!("{base}/api/v1/features/priority-support"))
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(created_feature["id"], support_id);

    let plan_version = PLAN_VERSION_2_LEETCODE_ID.as_base62();
    let entitlements_path = format!("{base}/api/v1/plan-versions/{plan_version}/entitlements");
    let batch = json!({
        "entitlements": [
            {
                "feature_id": support_id,
                "value": { "type": "BOOLEAN", "enabled": true },
            },
            {
                "feature_id": bandwidth_id,
                "value": {
                    "type": "METERED",
                    "limit": "500",
                    "reset_period": { "type": "BILLING_CYCLE" },
                    "enabled": true,
                },
            },
        ]
    });

    // A published version's grants are fixed: they are written through its draft.
    let published = client
        .post(format!(
            "{base}/api/v1/plan-versions/{}/entitlements",
            PLAN_VERSION_1_LEETCODE_ID.as_base62()
        ))
        .bearer_auth(&api_key)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(published.status(), reqwest::StatusCode::CONFLICT);

    let created = json_or_panic(
        client
            .post(&entitlements_path)
            .bearer_auth(&api_key)
            .json(&batch)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let created_rows = created["data"].as_array().unwrap();
    assert_eq!(created_rows.len(), 2);
    let created_feature_ids: Vec<&str> = created_rows
        .iter()
        .map(|e| e["feature_id"].as_str().unwrap())
        .collect();
    assert!(created_feature_ids.contains(&support_id.as_str()));
    assert!(created_feature_ids.contains(&bandwidth_id.as_str()));

    // Replaying the same batch skips what is already attached instead of duplicating it.
    let replayed = json_or_panic(
        client
            .post(&entitlements_path)
            .bearer_auth(&api_key)
            .json(&batch)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(replayed["data"].as_array().unwrap().len(), 0);

    let empty_batch = client
        .post(&entitlements_path)
        .bearer_auth(&api_key)
        .json(&json!({ "entitlements": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_batch.status(), reqwest::StatusCode::BAD_REQUEST);

    let listed = json_or_panic(
        client
            .get(&entitlements_path)
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;

    let support_entry = resolved_by_code(&listed, "priority-support");
    assert_eq!(support_entry["feature"]["id"], support_id);
    assert_eq!(support_entry["value"]["type"], "BOOLEAN");
    assert_eq!(support_entry["value"]["enabled"], true);

    let bandwidth_entry = resolved_by_code(&listed, "bandwidth");
    assert_eq!(bandwidth_entry["feature"]["id"], bandwidth_id);
    assert_eq!(bandwidth_entry["value"]["type"], "METERED");
    assert_eq!(bandwidth_entry["value"]["limit"], "500");
    assert_eq!(bandwidth_entry["value"]["enabled"], true);
    assert_eq!(
        bandwidth_entry["value"]["reset_period"]["type"],
        "BILLING_CYCLE"
    );
    assert_eq!(
        bandwidth_entry["value"]["metric_id"],
        METRIC_BANDWIDTH.as_base62()
    );

    // A second tenant, whose key must not reach the first tenant's plan version.
    let other_tenant = clients
        .tenants
        .clone()
        .create_tenant(api::tenants::v1::CreateTenantRequest {
            name: "other".to_string(),
            environment: api::tenants::v1::TenantEnvironmentEnum::Development as i32,
            disable_emails: None,
            email_delivery_mode: None,
            sandbox_preset: 0,
            copy_from: None,
        })
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    let other_clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        other_tenant.slug.as_str(),
    );
    let other_api_key = create_api_key(&other_clients, "rest-entitlement-writes-other").await;

    let cross_tenant_write = client
        .post(&entitlements_path)
        .bearer_auth(&other_api_key)
        .json(&json!({
            "entitlements": [
                {
                    "feature_id": support_id,
                    "value": { "type": "BOOLEAN", "enabled": true },
                },
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_tenant_write.status(), reqwest::StatusCode::NOT_FOUND);

    let cross_tenant_read = client
        .get(&entitlements_path)
        .bearer_auth(&other_api_key)
        .send()
        .await
        .unwrap();
    assert_eq!(cross_tenant_read.status(), reqwest::StatusCode::NOT_FOUND);

    // The rejected write left nothing behind on the first tenant's plan version.
    let after = json_or_panic(
        client
            .get(&entitlements_path)
            .bearer_auth(&api_key)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(
        after["data"].as_array().unwrap().len(),
        listed["data"].as_array().unwrap().len()
    );
}

/// A feature may only reference a product or a metric its own tenant owns, and a SELECT config
/// feature must ship with the options an entitlement can later be drawn from.
#[tokio::test]
async fn test_rest_create_feature_reference_validation() {
    let f = start_rest_fixture("rest-feature-refs").await;
    let other_key = second_tenant_key(
        &f,
        api::tenants::v1::TenantEnvironmentEnum::Staging,
        "refs-other",
        "rest-feature-refs-other",
    )
    .await;

    let families = json_or_panic(
        f.client
            .get(f.url("/api/v1/product_families"))
            .bearer_auth(&other_key)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let other_family_id = families["data"][0]["id"].as_str().unwrap().to_string();

    let foreign_product = json_or_panic(
        f.client
            .post(f.url("/api/v1/products"))
            .bearer_auth(&other_key)
            .json(&json!({
                "name": "Foreign product",
                "product_family_id": other_family_id,
                "fee_structure": { "type": "RATE" },
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let foreign_product_id = foreign_product["id"].as_str().unwrap().to_string();

    let foreign_metric = json_or_panic(
        f.client
            .post(f.url("/api/v1/metrics"))
            .bearer_auth(&other_key)
            .json(&json!({
                "name": "Foreign metric",
                "code": "foreign_metric",
                "aggregation_type": "COUNT",
                "product_family_id": other_family_id,
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let foreign_metric_id = foreign_metric["id"].as_str().unwrap().to_string();

    // A product owned by the other tenant must not become the new feature's product. It is a
    // body reference, so the miss is a 400 rather than the 404 reserved for the path resource.
    let rejected_product = f.post("/api/v1/features").json(&json!({
        "name": "Leaky product ref",
        "code": "leaky-product",
        "kind": "BOOLEAN",
        "product_id": foreign_product_id,
    }));
    assert_eq!(
        status_of(rejected_product).await,
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status_of(f.get("/api/v1/features/leaky-product")).await,
        reqwest::StatusCode::NOT_FOUND,
        "the rejected create must not have inserted a feature"
    );

    // Same for a metric owned by the other tenant.
    let rejected_metric = f.post("/api/v1/features").json(&json!({
        "name": "Leaky metric ref",
        "code": "leaky-metric",
        "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": foreign_metric_id },
    }));
    assert_eq!(
        status_of(rejected_metric).await,
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status_of(f.get("/api/v1/features/leaky-metric")).await,
        reqwest::StatusCode::NOT_FOUND,
        "the rejected create must not have inserted a feature"
    );

    let unknown_metric = f.post("/api/v1/features").json(&json!({
        "name": "Unknown metric ref",
        "code": "unknown-metric",
        "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": BillableMetricId::new().as_base62() },
    }));
    assert_eq!(
        status_of(unknown_metric).await,
        reqwest::StatusCode::BAD_REQUEST
    );

    // The tenant's own product and metric are still accepted.
    let owned = f
        .create_feature(json!({
            "name": "Owned bandwidth",
            "code": "owned-bandwidth",
            "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": METRIC_BANDWIDTH.as_base62() },
            "product_id": PRODUCT_PLATFORM_FEE_ID.as_base62(),
        }))
        .await;
    assert_eq!(owned["product"]["id"], PRODUCT_PLATFORM_FEE_ID.as_base62());
    assert_eq!(
        owned["feature_type"]["metric_id"],
        METRIC_BANDWIDTH.as_base62()
    );

    // A SELECT config feature with no options could never be satisfied.
    let empty_select = f.post("/api/v1/features").json(&json!({
        "name": "Support tier",
        "code": "support-tier",
        "kind": "VALUE", "value": { "type": "SELECT", "options": [] },
    }));
    assert_eq!(
        status_of(empty_select).await,
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status_of(f.get("/api/v1/features/support-tier")).await,
        reqwest::StatusCode::NOT_FOUND,
        "the rejected create must not have inserted a feature"
    );

    let tier = f
        .create_feature(json!({
            "name": "Support tier",
            "code": "support-tier",
            "kind": "VALUE", "value": { "type": "SELECT", "options": ["basic", "pro"] },
        }))
        .await;
    assert_eq!(tier["feature_type"]["value_type"], "SELECT");
    assert_eq!(tier["feature_type"]["options"], json!(["basic", "pro"]));

    // A non-SELECT config feature is unaffected by the options rule.
    let quota = f
        .create_feature(json!({
            "name": "Seat quota",
            "code": "seat-quota",
            "kind": "VALUE", "value": { "type": "NUMBER" },
        }))
        .await;
    assert_eq!(quota["feature_type"]["value_type"], "NUMBER");
}

/// `POST /products/{id}/entitlements` writes feature-level defaults, and rejects the whole
/// request — writing nothing — as soon as one spec targets a feature outside the product.
#[tokio::test]
async fn test_rest_product_and_add_on_entitlement_writes() {
    let f = start_rest_fixture("rest-product-addon-entitlements").await;
    let other_key = second_tenant_key(
        &f,
        api::tenants::v1::TenantEnvironmentEnum::Qa,
        "batch-other",
        "rest-product-addon-other",
    )
    .await;

    let platform = PRODUCT_PLATFORM_FEE_ID.as_base62();
    let product_path = format!("/api/v1/products/{platform}/entitlements");

    let sso = f
        .create_feature(json!({
            "name": "SSO",
            "code": "sso",
            "kind": "BOOLEAN",
            "product_id": platform,
        }))
        .await;
    let sso_id = sso["id"].as_str().unwrap().to_string();

    let created = json_or_panic(
        f.post(&product_path)
            .json(&json!({
                "entitlements": [
                    { "feature_id": sso_id, "value": { "type": "BOOLEAN", "enabled": true } },
                ]
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let created_rows = created["data"].as_array().unwrap();
    assert_eq!(created_rows.len(), 1);
    assert_eq!(created_rows[0]["feature_id"], sso_id);

    let listed = json_or_panic(
        f.get(&product_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let sso_entry = resolved_by_code(&listed, "sso");
    assert_eq!(sso_entry["value"]["type"], "BOOLEAN");
    assert_eq!(sso_entry["value"]["enabled"], true);

    // The write landed as the feature's own default entitlement.
    let stored = json_or_panic(
        f.get("/api/v1/features/sso").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(stored["entitlement"]["value"]["enabled"], true);

    // One batch, two specs: the second is out of the product's scope, so neither is written.
    let scoped = f
        .create_feature(json!({
            "name": "Audit log",
            "code": "audit-log",
            "kind": "BOOLEAN",
            "product_id": platform,
        }))
        .await;
    let scoped_id = scoped["id"].as_str().unwrap().to_string();
    let global = f
        .create_feature(json!({
            "name": "Global flag",
            "code": "global-flag",
            "kind": "BOOLEAN",
        }))
        .await;
    let global_id = global["id"].as_str().unwrap().to_string();

    let rejected = f.post(&product_path).json(&json!({
        "entitlements": [
            { "feature_id": scoped_id, "value": { "type": "BOOLEAN", "enabled": true } },
            { "feature_id": global_id, "value": { "type": "BOOLEAN", "enabled": true } },
        ]
    }));
    assert_eq!(status_of(rejected).await, reqwest::StatusCode::BAD_REQUEST);

    let scoped_after = json_or_panic(
        f.get("/api/v1/features/audit-log").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        scoped_after["entitlement"].is_null(),
        "the first spec of a rejected batch must not be committed: {scoped_after}"
    );
    let listed_after = json_or_panic(
        f.get(&product_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(find_resolved(&listed_after, "audit-log").is_none());
    assert_eq!(
        listed_after["data"].as_array().unwrap().len(),
        listed["data"].as_array().unwrap().len()
    );

    // The product belongs to the first tenant, so another tenant's key cannot write to it.
    assert_eq!(
        status_of(
            f.client
                .post(f.url(&product_path))
                .bearer_auth(&other_key)
                .json(&json!({
                    "entitlements": [
                        { "feature_id": sso_id, "value": { "type": "BOOLEAN", "enabled": true } },
                    ]
                }))
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );

    // ── Add-on entitlements ──
    let add_on = json_or_panic(
        f.post("/api/v1/addons")
            .json(&json!({
                "name": "Premium support",
                "product_id": platform,
                "price_id": PRICE_LEETCODE_RATE_ID.as_base62(),
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let add_on_id = add_on["id"].as_str().unwrap().to_string();
    let add_on_path = format!("/api/v1/addons/{add_on_id}/entitlements");

    let boost = f
        .create_feature(json!({
            "name": "Boost",
            "code": "boost",
            "kind": "BOOLEAN",
            "product_id": platform,
        }))
        .await;
    let boost_id = boost["id"].as_str().unwrap().to_string();

    let add_on_created = json_or_panic(
        f.post(&add_on_path)
            .json(&json!({
                "entitlements": [
                    { "feature_id": boost_id, "value": { "type": "BOOLEAN", "enabled": true } },
                ]
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(add_on_created["data"].as_array().unwrap().len(), 1);
    assert_eq!(add_on_created["data"][0]["feature_id"], boost_id);

    let add_on_listed = json_or_panic(
        f.get(&add_on_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let boost_entry = resolved_by_code(&add_on_listed, "boost");
    assert_eq!(boost_entry["feature"]["id"], boost_id);
    assert_eq!(boost_entry["value"]["enabled"], true);

    // Another tenant cannot attach entitlements to this add-on.
    assert_eq!(
        status_of(
            f.client
                .post(f.url(&add_on_path))
                .bearer_auth(&other_key)
                .json(&json!({
                    "entitlements": [
                        { "feature_id": boost_id, "value": { "type": "BOOLEAN", "enabled": true } },
                    ]
                }))
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
}

/// Single-entitlement update and delete, plus the feature patch and status lifecycle —
/// each checked against what the API reports afterwards, not just the status code.
#[tokio::test]
async fn test_rest_entitlement_update_delete_and_feature_lifecycle() {
    let f = start_rest_fixture("rest-entitlement-lifecycle").await;
    let other_key = second_tenant_key(
        &f,
        api::tenants::v1::TenantEnvironmentEnum::Demo,
        "lifecycle-other",
        "rest-lifecycle-other",
    )
    .await;

    let sso = f
        .create_feature(json!({
            "name": "SSO",
            "code": "sso",
            "description": "SAML",
            "kind": "BOOLEAN",
        }))
        .await;
    let sso_id = sso["id"].as_str().unwrap().to_string();
    let audit = f
        .create_feature(json!({
            "name": "Audit log",
            "code": "audit-log",
            "description": "retention",
            "kind": "BOOLEAN",
        }))
        .await;
    let audit_id = audit["id"].as_str().unwrap().to_string();

    let plan_version = PLAN_VERSION_2_LEETCODE_ID.as_base62();
    let plan_path = format!("/api/v1/plan-versions/{plan_version}/entitlements");
    let created = json_or_panic(
        f.post(&plan_path)
            .json(&json!({
                "entitlements": [
                    { "feature_id": sso_id, "value": { "type": "BOOLEAN", "enabled": true } },
                    { "feature_id": audit_id, "value": { "type": "BOOLEAN", "enabled": true } },
                ]
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let sso_entitlement_id = created["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["feature_id"] == sso_id.as_str())
        .expect("the batch created the SSO entitlement")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let entitlement_path = format!("/api/v1/entitlements/{sso_entitlement_id}");

    // ── Update ──
    let patched = json_or_panic(
        f.patch(&entitlement_path)
            .json(&json!({ "value": { "type": "BOOLEAN", "enabled": false } }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(patched["value"]["enabled"], false);
    assert_eq!(patched["feature_id"], sso_id);

    let read_back = json_or_panic(
        f.get(&entitlement_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(read_back["value"]["enabled"], false);
    let resolved = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(
        resolved_by_code(&resolved, "sso")["value"]["enabled"],
        false
    );

    // A value of the wrong variant for the feature is rejected and changes nothing.
    let mismatched = f.patch(&entitlement_path).json(&json!({
        "value": {
            "type": "METERED",
            "limit": "10",
            "reset_period": { "type": "NEVER" },
            "enabled": true,
        }
    }));
    assert_eq!(
        status_of(mismatched).await,
        reqwest::StatusCode::BAD_REQUEST
    );

    // Another tenant can neither read nor update it.
    assert_eq!(
        status_of(
            f.client
                .patch(f.url(&entitlement_path))
                .bearer_auth(&other_key)
                .json(&json!({ "value": { "type": "BOOLEAN", "enabled": true } }))
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    let unchanged = json_or_panic(
        f.get(&entitlement_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(unchanged["value"]["type"], "BOOLEAN");
    assert_eq!(unchanged["value"]["enabled"], false);

    let unknown_entitlement = format!("/api/v1/entitlements/{}", EntitlementId::new().as_base62());
    assert_eq!(
        status_of(
            f.patch(&unknown_entitlement)
                .json(&json!({ "value": { "type": "BOOLEAN", "enabled": true } }))
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );

    // ── Delete ──
    assert_eq!(
        status_of(f.delete(&unknown_entitlement)).await,
        reqwest::StatusCode::NOT_FOUND,
        "deleting an id that does not exist must not report success"
    );
    assert_eq!(
        status_of(
            f.client
                .delete(f.url(&entitlement_path))
                .bearer_auth(&other_key)
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(f.get(&entitlement_path)).await,
        reqwest::StatusCode::OK,
        "the cross-tenant delete must have left the row in place"
    );

    assert_eq!(
        status_of(f.delete(&entitlement_path)).await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        status_of(f.get(&entitlement_path)).await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(f.delete(&entitlement_path)).await,
        reqwest::StatusCode::NOT_FOUND,
        "a replayed delete is a 404 once the row is gone"
    );
    let after_delete = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(find_resolved(&after_delete, "sso").is_none());
    assert!(find_resolved(&after_delete, "audit-log").is_some());

    // ── Feature patch ──
    let renamed = json_or_panic(
        f.patch("/api/v1/features/audit-log")
            .json(&json!({ "name": "Audit trail" }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(renamed["name"], "Audit trail");

    let stored = json_or_panic(
        f.get("/api/v1/features/audit-log").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(stored["id"], audit_id);
    assert_eq!(stored["name"], "Audit trail");
    assert_eq!(stored["code"], "audit-log");
    assert_eq!(
        stored["description"], "retention",
        "a name-only patch leaves the description alone"
    );

    json_or_panic(
        f.patch("/api/v1/features/audit-log")
            .json(&json!({ "description": "7 years" }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let stored = json_or_panic(
        f.get("/api/v1/features/audit-log").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(stored["description"], "7 years");
    assert_eq!(stored["name"], "Audit trail");

    // An explicit null clears the description; the field is then omitted from the response.
    let cleared = json_or_panic(
        f.patch("/api/v1/features/audit-log")
            .json(&json!({ "description": null }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        cleared.get("description").is_none(),
        "a null description must clear it: {cleared}"
    );
    let stored = json_or_panic(
        f.get("/api/v1/features/audit-log").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(stored.get("description").is_none(), "{stored}");
    assert_eq!(stored["name"], "Audit trail");

    let empty_name = f
        .patch("/api/v1/features/audit-log")
        .json(&json!({ "name": "" }));
    assert_eq!(
        status_of(empty_name).await,
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status_of(
            f.patch("/api/v1/features/no-such-code")
                .json(&json!({ "name": "x" }))
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            f.client
                .patch(f.url("/api/v1/features/audit-log"))
                .bearer_auth(&other_key)
                .json(&json!({ "name": "hijacked" }))
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );
    let stored = json_or_panic(
        f.get("/api/v1/features/audit-log").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(stored["name"], "Audit trail");

    // ── Archive: hidden, code released ──
    assert_eq!(
        status_of(f.post("/api/v1/features/no-such-code/archive")).await,
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        status_of(
            f.client
                .post(f.url("/api/v1/features/audit-log/archive"))
                .bearer_auth(&other_key)
        )
        .await,
        reqwest::StatusCode::NOT_FOUND
    );

    assert_eq!(
        status_of(f.post("/api/v1/features/audit-log/archive")).await,
        reqwest::StatusCode::NO_CONTENT
    );
    let archived_path = format!("/api/v1/features/{audit_id}");
    let archived = json_or_panic(
        f.get(&archived_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(archived["status"], "ARCHIVED");
    assert_eq!(
        status_of(f.get("/api/v1/features/audit-log")).await,
        reqwest::StatusCode::NOT_FOUND,
        "an archived feature releases its code"
    );
    let listed = json_or_panic(
        f.get("/api/v1/features").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        listed.to_string().contains(&audit_id),
        "the REST listing without a filter still returns archived features"
    );
    let archived_only = json_or_panic(
        f.get("/api/v1/features?statuses=ARCHIVED")
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        archived_only.to_string().contains(&audit_id),
        "filtering by ARCHIVED lists them"
    );
    let while_archived = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        find_resolved(&while_archived, "audit-log").is_none(),
        "an archived feature's grants stop resolving: {while_archived}"
    );
    let other_version = PLAN_VERSION_PRO_DRAFT_ID.as_base62();
    assert_eq!(
        status_of(
            f.post(&format!(
                "/api/v1/plan-versions/{other_version}/entitlements"
            ))
            .json(&json!({
                "entitlements": [
                    { "feature_id": audit_id, "value": { "type": "BOOLEAN", "enabled": true } },
                ]
            }))
        )
        .await,
        reqwest::StatusCode::BAD_REQUEST,
        "an archived feature takes no new grants"
    );
    assert_eq!(
        status_of(f.patch(&archived_path).json(&json!({ "name": "renamed" }))).await,
        reqwest::StatusCode::BAD_REQUEST,
        "an archived feature is read-only"
    );

    // ── Unarchive is refused while another feature holds the code ──
    let recreated = f
        .create_feature(json!({
            "name": "Audit log v2",
            "code": "audit-log",
            "kind": "BOOLEAN",
        }))
        .await;
    assert_ne!(recreated["id"].as_str().unwrap(), audit_id);
    let after_recreate = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        find_resolved(&after_recreate, "audit-log").is_none(),
        "the archived feature's plan grant must not carry over to the new feature"
    );
    let refused = json_or_panic(
        f.post(&format!("{archived_path}/unarchive"))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CONFLICT,
    )
    .await;
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|m| m.contains("Audit log v2")),
        "the conflict names the feature holding the code: {refused}"
    );

    assert_eq!(
        status_of(f.post("/api/v1/features/audit-log/archive")).await,
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        status_of(f.post(&format!("{archived_path}/unarchive"))).await,
        reqwest::StatusCode::NO_CONTENT
    );
    let after_unarchive = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(
        resolved_by_code(&after_unarchive, "audit-log")["value"]["enabled"],
        true,
        "unarchiving brings the grants back"
    );
    let by_code = json_or_panic(
        f.get("/api/v1/features/audit-log").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(by_code["id"], audit_id);
    assert_eq!(
        status_of(f.post(&format!("{archived_path}/unarchive"))).await,
        reqwest::StatusCode::NO_CONTENT,
        "unarchiving an active feature is a no-op"
    );
}

/// A metered value is rejected with a 400 on every path that accepts one when its limit is
/// negative, a plan body naming the same feature twice is rejected before any write, and a Set to
/// row's policy override is validated and kept by a value update (ADR 0011 §3.1).
#[tokio::test]
async fn test_rest_entitlement_value_validation_and_update_carry_over() {
    use meteroid_store::domain::entitlement_fold::{GrantValue, LimitValue};
    use meteroid_store::domain::entitlement_policy::AtLimit;
    use meteroid_store::repositories::entitlements::EntitlementsInterface;

    let f = start_rest_fixture("rest-entitlement-validation").await;
    let platform = PRODUCT_PLATFORM_FEE_ID.as_base62();
    let metric = METRIC_BANDWIDTH.as_base62();

    let metered_value = |limit: &str| json!({ "type": "METERED", "limit": limit, "enabled": true });
    let negative_limit = metered_value("-1");

    // ── Feature create with a default entitlement ──
    for (code, value) in [("neg-limit", &negative_limit)] {
        let rejected = f.post("/api/v1/features").json(&json!({
            "name": code,
            "code": code,
            "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": metric },
            "entitlement": value,
        }));
        assert_eq!(
            status_of(rejected).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{code}"
        );
        assert_eq!(
            status_of(f.get(&format!("/api/v1/features/{code}"))).await,
            reqwest::StatusCode::NOT_FOUND,
            "the rejected create must not have inserted a feature"
        );
    }

    let bandwidth = f
        .create_feature(json!({
            "name": "Bandwidth",
            "code": "bandwidth",
            "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": metric },
            "product_id": platform,
        }))
        .await;
    let bandwidth_id = bandwidth["id"].as_str().unwrap().to_string();

    // ── Product / plan-version / add-on batch paths ──
    // The published v1 is reopened as its draft, so what a rejection left would resolve on it.
    let plan_version = PLAN_VERSION_1_LEETCODE_ID.as_base62();
    let plan_path = format!("/api/v1/plan-versions/{plan_version}/entitlements");
    let product_path = format!("/api/v1/products/{platform}/entitlements");
    helpers::plans::as_draft(&f.setup.store, PLAN_VERSION_1_LEETCODE_ID, async {
        for path in [&plan_path, &product_path] {
            for value in [&negative_limit] {
                let rejected = f.post(path).json(&json!({
                    "entitlements": [{ "feature_id": bandwidth_id, "value": value }]
                }));
                assert_eq!(
                    status_of(rejected).await,
                    reqwest::StatusCode::BAD_REQUEST,
                    "{path}: {value}"
                );
            }
        }
        // The same feature twice in one batch would race on the unique key: reject it up front.
        let duplicated = f.post(&plan_path).json(&json!({
            "entitlements": [
                { "feature_id": bandwidth_id, "value": metered_value("10") },
                { "feature_id": bandwidth_id, "value": metered_value("20") },
            ]
        }));
        assert_eq!(
            status_of(duplicated).await,
            reqwest::StatusCode::BAD_REQUEST
        );
    })
    .await;
    let stored = json_or_panic(
        f.get("/api/v1/features/bandwidth").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(stored["entitlement"].is_null(), "{stored}");
    let resolved = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        find_resolved(&resolved, "bandwidth").is_none(),
        "{resolved}"
    );

    // ── Plan create / replace bodies ──
    let plan_body = |entitlements: Value| {
        json!({
            "name": "Validated plan",
            "product_family_id": crate::data::ids::PRODUCT_FAMILY_ID.as_base62(),
            "plan_type": "FREE",
            "status": "ACTIVE",
            "currency": "EUR",
            "components": [],
            "entitlements": entitlements,
        })
    };
    let duplicate_specs = json!([
        { "feature_id": bandwidth_id, "value": metered_value("10") },
        { "feature_id": bandwidth_id, "value": metered_value("20") },
    ]);
    for entitlements in [
        json!([{ "feature_id": bandwidth_id, "value": negative_limit }]),
        duplicate_specs.clone(),
    ] {
        let rejected = f
            .post("/api/v1/plans")
            .json(&plan_body(entitlements.clone()));
        assert_eq!(
            status_of(rejected).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{entitlements}"
        );
    }
    let plans = json_or_panic(
        f.get("/api/v1/plans?search=Validated")
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        plans["data"].as_array().unwrap().is_empty(),
        "a rejected plan body must not have created a plan: {plans}"
    );

    // A trial is free: a billed one is refused, a free one read back as written.
    let mut trialed = plan_body(json!([]));
    trialed["name"] = json!("Trialed plan");
    trialed["trial"] = json!({"duration_days": 14, "is_free": false, "trialing_plan_id": null});
    let refused = f.post("/api/v1/plans").json(&trialed).send().await.unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    let refusal: Value = refused.json().await.unwrap();
    assert_eq!(
        refusal["message"],
        meteroid_store::domain::trials::TRIAL_IS_FREE,
        "{refusal}"
    );
    trialed["trial"]["is_free"] = json!(true);
    let free = json_or_panic(
        f.post("/api/v1/plans").json(&trialed).send().await.unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(free["trial"]["is_free"], true, "{free}");
    assert_eq!(free["trial"]["duration_days"], 14);

    let created = json_or_panic(
        f.post("/api/v1/plans")
            .json(&plan_body(json!([
                { "feature_id": bandwidth_id, "value": metered_value("10") }
            ])))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let plan_id = created["id"].as_str().unwrap().to_string();
    let replace_path = format!("/api/v1/plans/{plan_id}");
    for entitlements in [duplicate_specs] {
        let rejected = f.put(&replace_path).json(&plan_body(entitlements.clone()));
        assert_eq!(
            status_of(rejected).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{entitlements}"
        );
    }
    let unchanged = json_or_panic(
        f.get(&replace_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(unchanged["version_id"], created["version_id"]);

    // The plan is published: its rows change through a draft (ADR-0011 §3.2).
    let version_id =
        common_domain::ids::PlanVersionId::parse_base62(created["version_id"].as_str().unwrap())
            .unwrap();
    helpers::plans::as_draft(&f.setup.store, version_id, async {
        // ── Update: invalid values rejected, overage settings carried over ──
        let entitlement_id = created["entitlements"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let entitlement_path = format!("/api/v1/entitlements/{entitlement_id}");
        for value in [&negative_limit] {
            let rejected = f.patch(&entitlement_path).json(&json!({ "value": value }));
            assert_eq!(
                status_of(rejected).await,
                reqwest::StatusCode::BAD_REQUEST,
                "{value}"
            );
        }

        let store = &f.setup.store;
        let stored_id = EntitlementId::parse_base62(&entitlement_id).unwrap();
        let patch_ok = async |body: Value| {
            json_or_panic(
                f.patch(&entitlement_path).json(&body).send().await.unwrap(),
                reqwest::StatusCode::OK,
            )
            .await
        };

        // ── A Set to row's policy override, kept by a value update ──
        let terms = json!({ "resets": "BILLING_CYCLE", "at_limit": "ALLOW_AND_BILL", "grace_pct": 0, "thresholds": [80] });
        let overridden = patch_ok(json!({ "policy_override": terms })).await;
        assert_eq!(overridden["policy_override"], terms, "{overridden}");
        let patched = patch_ok(json!({ "value": metered_value("25") })).await;
        assert_eq!(patched["value"]["limit"], "25");
        assert_eq!(
            patched["policy_override"], terms,
            "a value update keeps the override: {patched}"
        );
        let stored = store.get_entitlement(stored_id, TENANT_ID).await.unwrap();
        assert_eq!(
            stored.value,
            GrantValue::Limit(LimitValue::Amount(dec!(25)))
        );
        assert_eq!(
            stored.policy_override.map(|p| p.at_limit),
            Some(AtLimit::AllowAndBill)
        );

        // An hourly calendar window keeps its unit (ADR 0011 §3.1).
        let hourly = json!({ "resets": "CALENDAR", "period": { "unit": "HOUR", "interval": 1 }, "at_limit": "DENY", "grace_pct": 0, "thresholds": [80] });
        let windowed = patch_ok(json!({ "policy_override": hourly })).await;
        assert_eq!(windowed["policy_override"], hourly, "{windowed}");
        assert_eq!(
            store
                .get_entitlement(stored_id, TENANT_ID)
                .await
                .unwrap()
                .policy_override
                .unwrap()
                .reset_period(true),
            meteroid_store::domain::entitlements::ResetPeriod::Calendar {
                unit: meteroid_store::domain::entitlements::PeriodUnit::Hour,
                interval: 1
            }
        );

        let too_many = (1..=11).collect::<Vec<u32>>();
        for (terms, why) in [
            (
                json!({ "resets": "BILLING_CYCLE", "at_limit": "ALLOW_AND_BILL", "grace_pct": 10 }),
                "a grace applies to DENY only",
            ),
            (
                json!({ "resets": "SLIDING", "period": { "unit": "DAY", "interval": 5 }, "at_limit": "DENY", "thresholds": [80] }),
                "a sliding window takes no thresholds",
            ),
            (
                json!({ "resets": "BILLING_CYCLE", "at_limit": "DENY", "thresholds": too_many }),
                "at most ten thresholds",
            ),
            (
                json!({ "resets": "FIXED", "at_limit": "DENY" }),
                "a fixed window needs a period",
            ),
            (
                json!({ "resets": "CALENDAR", "period": { "unit": "WEEK", "interval": 0 }, "at_limit": "DENY" }),
                "a calendar window needs a positive interval",
            ),
            (
                json!({ "resets": "BILLING_CYCLE", "at_limit": "DENY_NEW" }),
                "DENY_NEW is for limits counted by the app",
            ),
        ] {
            let rejected = f
                .patch(&entitlement_path)
                .json(&json!({ "policy_override": terms }));
            assert_eq!(
                status_of(rejected).await,
                reqwest::StatusCode::BAD_REQUEST,
                "{why}"
            );
        }
        let raised = f
            .patch(&entitlement_path)
            .json(&json!({ "effect": "RAISE_TO" }));
        assert_eq!(
            status_of(raised).await,
            reqwest::StatusCode::BAD_REQUEST,
            "a plan row is a Set to, which alone carries an override"
        );
        let cleared = patch_ok(json!({ "policy_override": null })).await;
        assert!(
            cleared.get("policy_override").is_none(),
            "null removes the override: {cleared}"
        );
    })
    .await;
}

/// Replacing a published plan mints a new version, and entitlements hang off a version, so a
/// plan body that could not carry them left every replace needing a second attach call.
#[tokio::test]
async fn test_rest_plan_body_carries_entitlements_across_versions() {
    let f = start_rest_fixture("rest-plan-entitlements").await;

    let sso = f
        .create_feature(json!({
            "name": "Single sign-on",
            "code": "plan_body_sso",
            "kind": "BOOLEAN"
        }))
        .await;
    let sso_id = sso["id"].as_str().unwrap().to_string();

    let plan_body = |enabled: bool| {
        json!({
            "name": "Carries Entitlements",
            "product_family_id": crate::data::ids::PRODUCT_FAMILY_ID.as_base62(),
            "plan_type": "FREE",
            "status": "ACTIVE",
            "currency": "EUR",
            "components": [],
            "entitlements": [
                { "feature_id": sso_id, "value": { "type": "BOOLEAN", "enabled": enabled } }
            ]
        })
    };

    let created = json_or_panic(
        f.post("/api/v1/plans")
            .json(&plan_body(true))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;

    let plan_id = created["id"].as_str().unwrap().to_string();
    let first_version = created["version_id"].as_str().unwrap().to_string();
    let attached = created["entitlements"]
        .as_array()
        .expect("the plan carries its entitlements");
    assert_eq!(attached.len(), 1, "{created}");
    assert_eq!(attached[0]["feature_id"], json!(sso_id));
    assert_eq!(attached[0]["value"]["enabled"], json!(true));

    let replaced = json_or_panic(
        f.put(&format!("/api/v1/plans/{plan_id}"))
            .json(&plan_body(false))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;

    let second_version = replaced["version_id"].as_str().unwrap().to_string();
    assert_ne!(
        second_version, first_version,
        "replacing a published plan mints a new version"
    );

    let carried = replaced["entitlements"]
        .as_array()
        .expect("the replacement carries its entitlements");
    assert_eq!(
        carried.len(),
        1,
        "the new version carries the entitlements the replace body named: {replaced}"
    );
    assert_eq!(carried[0]["feature_id"], json!(sso_id));
    assert_eq!(carried[0]["value"]["enabled"], json!(false));
    assert_ne!(
        carried[0]["id"], attached[0]["id"],
        "they are new rows on the new version, not the old version's"
    );

    let refetched = json_or_panic(
        f.get(&format!("/api/v1/plans/{plan_id}"))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(refetched["version_id"], json!(second_version));
    assert_eq!(
        refetched["entitlements"].as_array().map(|e| e.len()),
        Some(1),
        "a re-read of the plan still reports them: {refetched}"
    );

    assert_eq!(
        status_of(f.post("/api/v1/features/plan_body_sso/archive")).await,
        reqwest::StatusCode::NO_CONTENT
    );
    json_or_panic(
        f.put(&format!("/api/v1/plans/{plan_id}"))
            .json(&plan_body(true))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let mut new_plan = plan_body(true);
    new_plan["name"] = json!("Grants Archived");
    let created = json_or_panic(
        f.post("/api/v1/plans")
            .json(&new_plan)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert!(
        created.get("entitlements").is_none(),
        "a new plan skips the archived feature's grant: {created}"
    );

    // A version is a snapshot of what its author could see, so the version the replace made while
    // the feature was archived holds neither the skipped body grant nor the hidden one.
    assert_eq!(
        status_of(f.post(&format!("/api/v1/features/{sso_id}/unarchive"))).await,
        reqwest::StatusCode::NO_CONTENT
    );
    let restored = json_or_panic(
        f.get(&format!("/api/v1/plans/{plan_id}"))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert!(
        restored.get("entitlements").is_none(),
        "the version made while archived stays without the grant: {restored}"
    );
}

/// A feature granted on every kind of entity is archived: each entity stays readable,
/// resolvable and copyable with no 400 or 500, and the archived feature is simply absent. Once
/// restored it is back on every entity that held it, but not on entities made while it was
/// archived.
#[tokio::test]
async fn test_rest_archived_feature_drops_out_and_comes_back() {
    use crate::data::ids::{CUST_UBER_ID, PLAN_LEETCODE_ID};
    use api::entitlements::v1::{
        CreateEntitlementRequest, EntitlementEntity, EntitlementSpec, EntitlementValue,
        GetEffectiveEntitlementsRequest, GetResolvedForAddOnRequest,
        GetResolvedForPlanVersionRequest, GetResolvedForProductRequest, GetResolvedForQuoteRequest,
        GetResolvedForSubscriptionRequest, ListEntitlementsByEntityRequest, entitlement_entity,
        entitlement_value,
    };
    use common_domain::ids::{AddOnId, FeatureId, PlanId, PlanVersionId, QuoteId, SubscriptionId};
    use meteroid_store::repositories::QuotesInterface;

    let f = start_rest_fixture("rest-archived-feature").await;
    let platform = PRODUCT_PLATFORM_FEE_ID.as_base62();
    let grant =
        |id: &str| json!({ "feature_id": id, "value": { "type": "BOOLEAN", "enabled": true } });

    let shelved_id = f
        .create_feature(json!({
            "name": "Shelved",
            "code": "shelved",
            "kind": "BOOLEAN",
            "product_id": platform,
        }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let kept_id = f
        .create_feature(json!({
            "name": "Kept",
            "code": "kept",
            "kind": "BOOLEAN",
            "product_id": platform,
        }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let shelved_proto = FeatureId::parse_base62(&shelved_id).unwrap().as_proto();
    let kept_proto = FeatureId::parse_base62(&kept_id).unwrap().as_proto();
    let both = json!({ "entitlements": [grant(&shelved_id), grant(&kept_id)] });

    // ── Grant both features on every kind of entity ──
    json_or_panic(
        f.post(&format!("/api/v1/products/{platform}/entitlements"))
            .json(&both)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;

    let plan_body = json!({
        "name": "Grants shelved",
        "product_family_id": crate::data::ids::PRODUCT_FAMILY_ID.as_base62(),
        "plan_type": "FREE",
        "status": "ACTIVE",
        "currency": "EUR",
        "components": [],
        "entitlements": [grant(&shelved_id), grant(&kept_id)],
    });
    let plan = json_or_panic(
        f.post("/api/v1/plans")
            .json(&plan_body)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let plan_id = plan["id"].as_str().unwrap().to_string();
    let plan_version = plan["version_id"].as_str().unwrap().to_string();

    let mut plan_version_grant_id = String::new();
    // The published v1 is written as a fixture of the catalog; v2 is its draft.
    for version in [PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_2_LEETCODE_ID] {
        let created = helpers::plans::as_draft(&f.setup.store, version, async {
            json_or_panic(
                f.post(&format!(
                    "/api/v1/plan-versions/{}/entitlements",
                    version.as_base62()
                ))
                .json(&both)
                .send()
                .await
                .unwrap(),
                reqwest::StatusCode::CREATED,
            )
            .await
        })
        .await;
        plan_version_grant_id = created["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["feature_id"] == shelved_id.as_str())
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
    }

    let add_on_id = json_or_panic(
        f.post("/api/v1/addons")
            .json(&json!({
                "name": "Shelved add-on",
                "product_id": platform,
                "price_id": PRICE_LEETCODE_RATE_ID.as_base62(),
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    json_or_panic(
        f.post(&format!("/api/v1/addons/{add_on_id}/entitlements"))
            .json(&both)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;

    let subscription_id = json_or_panic(
        f.post("/api/v1/subscriptions")
            .json(&json!({
                "plan_id": PLAN_LEETCODE_ID.as_base62(),
                "customer_id_or_alias": CUST_UBER_ID.as_base62(),
                "start_date": chrono::Utc::now().date_naive(),
                "activation_condition": "ON_START",
                "charge_automatically": false,
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let subscription_proto = SubscriptionId::parse_base62(&subscription_id)
        .unwrap()
        .as_proto();

    let grpc_grant = |feature_id: String, entity: entitlement_entity::EntityId| {
        let mut client = f.clients.entitlements.clone();
        async move {
            client
                .create_entitlement(CreateEntitlementRequest {
                    entity: Some(EntitlementEntity {
                        entity_id: Some(entity),
                    }),
                    spec: Some(EntitlementSpec {
                        feature_id,
                        value: Some(EntitlementValue {
                            value: Some(entitlement_value::Value::BooleanValue(
                                entitlement_value::BooleanValue { enabled: true },
                            )),
                        }),
                        ..Default::default()
                    }),
                    resource_id: None,
                })
                .await
                .unwrap();
        }
    };
    for feature in [&shelved_proto, &kept_proto] {
        grpc_grant(
            feature.clone(),
            entitlement_entity::EntityId::SubscriptionId(subscription_proto.clone()),
        )
        .await;
        grpc_grant(
            feature.clone(),
            entitlement_entity::EntityId::CustomerId(CUST_UBER_ID.as_proto()),
        )
        .await;
    }

    // A quote agreed before the archive, granting both features.
    let mut quote_request = crate::test_quote::quote_granting(&shelved_proto);
    if let Some(quote) = quote_request.quote.as_mut() {
        let mut kept_spec = quote.entitlements[0].clone();
        kept_spec.feature_id = kept_proto.clone();
        quote.entitlements.push(kept_spec);
    }
    let quote_proto = f
        .clients
        .quotes
        .clone()
        .create_quote(tonic::Request::new(quote_request))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap()
        .quote
        .unwrap()
        .id;
    f.clients
        .quotes
        .clone()
        .publish_quote(tonic::Request::new(api::quotes::v1::PublishQuoteRequest {
            id: quote_proto.clone(),
        }))
        .await
        .unwrap();
    f.setup
        .store
        .accept_quote(
            common_domain::actor::Actor::System,
            QuoteId::from_proto(&quote_proto).unwrap(),
            TENANT_ID,
        )
        .await
        .unwrap();

    // ── Archive ──
    assert_eq!(
        status_of(f.post("/api/v1/features/shelved/archive")).await,
        reqwest::StatusCode::NO_CONTENT
    );

    // ── REST reads: 200, without the archived feature, with the kept one ──
    let reads = [
        format!("/api/v1/products/{platform}/entitlements"),
        format!("/api/v1/plans/{plan_id}"),
        format!(
            "/api/v1/plan-versions/{}/entitlements",
            PLAN_VERSION_1_LEETCODE_ID.as_base62()
        ),
        format!("/api/v1/addons/{add_on_id}"),
        format!("/api/v1/addons/{add_on_id}/entitlements"),
        format!("/api/v1/subscriptions/{subscription_id}"),
        format!("/api/v1/subscriptions/{subscription_id}/entitlements"),
        format!(
            "/api/v1/customers/{}/entitlements",
            CUST_UBER_ID.as_base62()
        ),
    ];
    for path in &reads {
        let body = json_or_panic(f.get(path).send().await.unwrap(), reqwest::StatusCode::OK)
            .await
            .to_string();
        assert!(
            !body.contains(&shelved_id),
            "{path} still returns the archived feature: {body}"
        );
        assert!(
            body.contains(&kept_id),
            "{path} lost the other feature: {body}"
        );
    }
    // The new plan has no components, so its product-scoped features don't resolve on it either way.
    let new_plan_resolved = json_or_panic(
        f.get(&format!(
            "/api/v1/plan-versions/{plan_version}/entitlements"
        ))
        .send()
        .await
        .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await
    .to_string();
    assert!(
        !new_plan_resolved.contains(&shelved_id),
        "{new_plan_resolved}"
    );
    let features = json_or_panic(
        f.get("/api/v1/features?statuses=ACTIVE&statuses=DISABLED")
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await
    .to_string();
    assert!(!features.contains(&shelved_id), "{features}");
    assert_eq!(
        status_of(f.get(&format!("/api/v1/entitlements/{plan_version_grant_id}"))).await,
        reqwest::StatusCode::NOT_FOUND,
        "an archived feature's grant reads as not found"
    );
    assert_eq!(
        status_of(f.delete(&format!("/api/v1/entitlements/{plan_version_grant_id}"))).await,
        reqwest::StatusCode::NOT_FOUND,
        "an archived feature's grant cannot be removed"
    );

    // ── gRPC resolution and listings on every entity ──
    let ec = || f.clients.entitlements.clone();
    let plan_version_proto = PlanVersionId::parse_base62(&plan_version)
        .unwrap()
        .as_proto();
    let add_on_proto = AddOnId::parse_base62(&add_on_id).unwrap().as_proto();
    let resolved: Vec<(&str, String)> = vec![
        (
            "product",
            format!(
                "{:?}",
                ec().get_resolved_entitlements_for_product(GetResolvedForProductRequest {
                    product_id: PRODUCT_PLATFORM_FEE_ID.as_proto(),
                })
                .await
                .unwrap()
            ),
        ),
        (
            "plan version",
            format!(
                "{:?}",
                ec().get_resolved_entitlements_for_plan_version(GetResolvedForPlanVersionRequest {
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID.as_proto(),
                })
                .await
                .unwrap()
            ),
        ),
        (
            "add-on",
            format!(
                "{:?}",
                ec().get_resolved_entitlements_for_add_on(GetResolvedForAddOnRequest {
                    add_on_id: add_on_proto.clone(),
                })
                .await
                .unwrap()
            ),
        ),
        (
            "subscription",
            format!(
                "{:?}",
                ec().get_resolved_entitlements_for_subscription(
                    GetResolvedForSubscriptionRequest {
                        subscription_id: subscription_proto.clone(),
                    }
                )
                .await
                .unwrap()
            ),
        ),
        (
            "quote",
            format!(
                "{:?}",
                ec().get_resolved_entitlements_for_quote(GetResolvedForQuoteRequest {
                    quote_id: quote_proto.clone(),
                })
                .await
                .unwrap()
            ),
        ),
        (
            "customer",
            format!(
                "{:?}",
                ec().get_effective_entitlements(GetEffectiveEntitlementsRequest {
                    customer_id: CUST_UBER_ID.as_proto(),
                })
                .await
                .unwrap()
            ),
        ),
    ];
    for entity in [
        entitlement_entity::EntityId::FeatureId(shelved_proto.clone()),
        entitlement_entity::EntityId::PlanVersionId(plan_version_proto.clone()),
        entitlement_entity::EntityId::AddOnId(add_on_proto.clone()),
        entitlement_entity::EntityId::SubscriptionId(subscription_proto.clone()),
        entitlement_entity::EntityId::QuoteId(quote_proto.clone()),
        entitlement_entity::EntityId::CustomerId(CUST_UBER_ID.as_proto()),
    ] {
        let label = format!("{entity:?}");
        let listed = ec()
            .list_entitlements_by_entity(ListEntitlementsByEntityRequest {
                entity: Some(EntitlementEntity {
                    entity_id: Some(entity),
                }),
            })
            .await
            .unwrap_or_else(|e| panic!("listing grants on {label}: {e:?}"))
            .into_inner();
        assert!(
            listed
                .entitlements
                .iter()
                .all(|e| e.feature_id != shelved_proto),
            "grants listed on {label} include the archived feature"
        );
    }
    for (label, body) in &resolved {
        assert!(
            !body.contains(&shelved_proto),
            "{label} resolves the archived feature"
        );
    }
    for (label, body) in resolved
        .iter()
        .filter(|(l, _)| *l != "quote" && *l != "add-on")
    {
        assert!(body.contains(&kept_proto), "{label} lost the other feature");
    }

    // ── Copies of entities that granted the archived feature still succeed ──
    let replaced = json_or_panic(
        f.put(&format!("/api/v1/plans/{plan_id}"))
            .json(&plan_body)
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let replaced_body = replaced.to_string();
    assert!(!replaced_body.contains(&shelved_id), "{replaced_body}");
    assert!(replaced_body.contains(&kept_id), "{replaced_body}");

    let draft_proto = f
        .clients
        .plans
        .clone()
        .copy_version_to_draft(api::plans::v1::CopyVersionToDraftRequest {
            plan_id: PlanId::parse_base62(&plan_id).unwrap().as_proto(),
            plan_version_id: plan_version_proto.clone(),
        })
        .await
        .expect("a plan version that granted an archived feature can be copied to a draft")
        .into_inner()
        .plan_version
        .unwrap()
        .id;

    let converted = f
        .clients
        .quotes
        .clone()
        .convert_quote_to_subscription(tonic::Request::new(
            api::quotes::v1::ConvertQuoteToSubscriptionRequest {
                quote_id: quote_proto.clone(),
            },
        ))
        .await
        .expect("a quote granting an archived feature still converts")
        .into_inner()
        .subscription
        .unwrap()
        .id;
    let listed_on = |entity: entitlement_entity::EntityId| async move {
        format!(
            "{:?}",
            ec().list_entitlements_by_entity(ListEntitlementsByEntityRequest {
                entity: Some(EntitlementEntity {
                    entity_id: Some(entity),
                }),
            })
            .await
            .unwrap()
        )
    };
    let converted_grants = listed_on(entitlement_entity::EntityId::SubscriptionId(
        converted.clone(),
    ))
    .await;
    assert!(
        !converted_grants.contains(&shelved_proto),
        "{converted_grants}"
    );
    assert!(converted_grants.contains(&kept_proto), "{converted_grants}");

    // ── Restore: back on every entity that held it ──
    assert_eq!(
        status_of(f.post(&format!("/api/v1/features/{shelved_id}/unarchive"))).await,
        reqwest::StatusCode::NO_CONTENT
    );
    // The plan now reads its replaced version, which like the draft was made while archived.
    let plan_path = format!("/api/v1/plans/{plan_id}");
    for path in reads.iter().filter(|p| **p != plan_path) {
        let body = json_or_panic(f.get(path).send().await.unwrap(), reqwest::StatusCode::OK)
            .await
            .to_string();
        assert!(
            body.contains(&shelved_id),
            "{path} lacks the restored feature: {body}"
        );
    }
    for entity in [
        entitlement_entity::EntityId::FeatureId(shelved_proto.clone()),
        entitlement_entity::EntityId::PlanVersionId(plan_version_proto.clone()),
        entitlement_entity::EntityId::AddOnId(add_on_proto.clone()),
        entitlement_entity::EntityId::SubscriptionId(subscription_proto.clone()),
        entitlement_entity::EntityId::QuoteId(quote_proto.clone()),
        entitlement_entity::EntityId::CustomerId(CUST_UBER_ID.as_proto()),
    ] {
        let label = format!("{entity:?}");
        let listed = listed_on(entity).await;
        assert!(
            listed.contains(&shelved_proto),
            "{label} lacks the restored grant: {listed}"
        );
    }
    let replaced_plan = json_or_panic(
        f.get(&plan_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await
    .to_string();
    assert!(
        !replaced_plan.contains(&shelved_id),
        "a version made while archived stays as made: {replaced_plan}"
    );
    let draft_grants = listed_on(entitlement_entity::EntityId::PlanVersionId(draft_proto)).await;
    assert!(
        !draft_grants.contains(&shelved_proto),
        "a draft copied while archived stays as copied: {draft_grants}"
    );
    let converted_grants = listed_on(entitlement_entity::EntityId::SubscriptionId(converted)).await;
    assert!(
        !converted_grants.contains(&shelved_proto),
        "a subscription converted while archived stays as converted: {converted_grants}"
    );
    let restored = format!(
        "{:?}",
        ec().get_effective_entitlements(GetEffectiveEntitlementsRequest {
            customer_id: CUST_UBER_ID.as_proto(),
        })
        .await
        .unwrap()
    );
    assert!(
        restored.contains(&shelved_proto),
        "the customer gets the restored feature again"
    );
}

/// An archived feature still has its code, so unarchiving by code finds it, unless a non-archived
/// feature now holds the code or several archived features used it.
#[tokio::test]
async fn test_rest_unarchive_by_code() {
    let f = start_rest_fixture("rest-unarchive-by-code").await;
    let create = |code: &'static str| {
        let f = &f;
        async move {
            f.create_feature(json!({
                "name": code,
                "code": code,
                "kind": "BOOLEAN"
            }))
            .await["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let archive = |id: String| {
        let f = &f;
        async move {
            assert_eq!(
                status_of(f.post(&format!("/api/v1/features/{id}/archive"))).await,
                reqwest::StatusCode::NO_CONTENT
            );
        }
    };
    let unarchive = |code: &'static str| {
        let f = &f;
        async move { status_of(f.post(&format!("/api/v1/features/{code}/unarchive"))).await }
    };

    let first = create("by_code").await;
    archive(first.clone()).await;
    assert_eq!(
        unarchive("by_code").await,
        reqwest::StatusCode::NO_CONTENT,
        "the one archived feature with the code is unarchived"
    );
    assert_eq!(
        unarchive("by_code").await,
        reqwest::StatusCode::CONFLICT,
        "the code now belongs to a non-archived feature"
    );

    archive(first).await;
    let second = create("by_code").await;
    archive(second.clone()).await;
    assert_eq!(
        unarchive("by_code").await,
        reqwest::StatusCode::CONFLICT,
        "several archived features used the code"
    );
    assert_eq!(
        status_of(f.post(&format!("/api/v1/features/{second}/unarchive"))).await,
        reqwest::StatusCode::NO_CONTENT,
        "the ID still picks one"
    );

    assert_eq!(
        unarchive("no_such_code").await,
        reqwest::StatusCode::NOT_FOUND
    );
}

/// Usage priced by dimension at rates a matrix can hold reads back as MATRIX, as it did before
/// rates by dimension; rates no matrix holds read back as DIMENSION_RATES.
#[tokio::test]
async fn test_rest_plan_returns_matrix_shaped_rates_as_matrix() {
    let f = start_rest_fixture("rest-plan-matrix").await;
    let usage = |name: &str, pricing: Value| {
        json!({
            "name": name,
            "fee": {
                "type": "USAGE",
                "metric_id": METRIC_BANDWIDTH.as_base62(),
                "pricing": pricing,
                "cadence": "MONTHLY"
            }
        })
    };
    let created = json_or_panic(
        f.post("/api/v1/plans")
            .json(&json!({
                "name": "Matrix Back Compat",
                "product_family_id": crate::data::ids::PRODUCT_FAMILY_ID.as_base62(),
                "plan_type": "STANDARD",
                "status": "ACTIVE",
                "currency": "EUR",
                "components": [
                    usage("Regions", json!({
                        "type": "MATRIX",
                        "rates": [
                            {
                                "dimension1": {"key": "region", "value": "eu"},
                                "dimension2": {"key": "tier", "value": "gold"},
                                "per_unit_price": "0.20"
                            },
                            {
                                "dimension1": {"key": "region", "value": "us"},
                                "dimension2": null,
                                "per_unit_price": "0.10"
                            }
                        ]
                    })),
                    usage("Models", json!({
                        "type": "DIMENSION_RATES",
                        "dimensions": ["model"],
                        "rates": [{"key": ["gpt-4"], "rate": "3"}]
                    })),
                    usage("Tokens", json!({
                        "type": "DIMENSION_RATES",
                        "dimensions": ["model", "token_type"],
                        "rates": [{"key": ["gpt-*", "*"], "rate": "2.50"}],
                        "unit_size": "1000000"
                    }))
                ]
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let plan_id = created["id"].as_str().unwrap().to_string();
    let plan = json_or_panic(
        f.get(&format!("/api/v1/plans/{plan_id}"))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let pricing = |name: &str| {
        plan["price_components"]
            .as_array()
            .expect("the plan's components")
            .iter()
            .find(|component| component["name"] == name)
            .unwrap_or_else(|| panic!("no component {name} in {plan}"))["fee"]["pricing"]
            .clone()
    };
    // Each cell as (dimension1, dimension2, per unit price).
    let cells = |pricing: &Value| -> Vec<(Value, Value, rust_decimal::Decimal)> {
        pricing["rates"]
            .as_array()
            .expect("the matrix's cells")
            .iter()
            .map(|row| {
                (
                    row["dimension1"].clone(),
                    row["dimension2"].clone(),
                    row["per_unit_price"].as_str().unwrap().parse().unwrap(),
                )
            })
            .collect()
    };

    let regions = pricing("Regions");
    assert_eq!(regions["type"], "MATRIX", "{regions}");
    assert_eq!(
        cells(&regions),
        vec![
            (
                json!({"key": "region", "value": "eu"}),
                json!({"key": "tier", "value": "gold"}),
                dec!(0.20)
            ),
            (
                json!({"key": "region", "value": "us"}),
                Value::Null,
                dec!(0.10)
            ),
        ]
    );
    let models = pricing("Models");
    assert_eq!(models["type"], "MATRIX", "{models}");
    assert_eq!(
        cells(&models),
        vec![(
            json!({"key": "model", "value": "gpt-4"}),
            Value::Null,
            dec!(3)
        )]
    );
    let tokens = pricing("Tokens");
    assert_eq!(tokens["type"], "DIMENSION_RATES", "{tokens}");
    assert_eq!(tokens["dimensions"], json!(["model", "token_type"]));
}

/// ADR 0011 §8: `as_of` and `group[...]` were dropped; the check refuses them rather than
/// answering as if they were not there.
#[tokio::test]
async fn test_rest_check_refuses_dropped_inputs() {
    use crate::data::ids::CUST_UBER_ID;
    let f = start_rest_fixture("rest-check-dropped").await;
    f.create_feature(json!({ "name": "Audit log", "code": "audit_log", "kind": "BOOLEAN" }))
        .await;
    let customer = CUST_UBER_ID.as_base62();
    let post = format!("/api/v1/customers/{customer}/entitlements/check");
    let get = format!("/api/v1/customers/{customer}/entitlements/audit_log/check");
    assert_eq!(
        status_of(f.post(&post).json(&json!({ "feature": "audit_log" }))).await,
        reqwest::StatusCode::OK
    );
    for body in [
        json!({ "feature": "audit_log", "as_of": "2026-01-01T00:00:00Z" }),
        json!({ "feature": "audit_log", "group": { "region": "eu" } }),
    ] {
        assert_eq!(
            status_of(f.post(&post).json(&body)).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{body}"
        );
    }
    for query in ["group[region]=eu", "as_of=2026-01-01T00:00:00Z"] {
        assert_eq!(
            status_of(f.get(&format!("{get}?{query}"))).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    assert_eq!(status_of(f.get(&get)).await, reqwest::StatusCode::OK);
}

/// v1 published per-row limit terms until 2.3; they are the feature's policy now. A Set to row
/// takes the terms differing from it as its override, as the entitlements_v2 migration converts
/// rows. Any other row refuses them unless they match, and `group_by` is refused.
#[tokio::test]
async fn test_rest_v1_limit_terms_are_translated_or_refused() {
    use crate::data::ids::CUST_UBER_ID;

    let f = start_rest_fixture("rest-v1-limit-terms").await;
    let platform = PRODUCT_PLATFORM_FEE_ID.as_base62();
    let metric = METRIC_BANDWIDTH.as_base62();
    let api_calls = f
        .create_feature(json!({
            "name": "API calls",
            "code": "api-calls",
            "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": metric },
        }))
        .await;
    let default_policy =
        json!({ "resets": "BILLING_CYCLE", "at_limit": "DENY", "grace_pct": 0, "thresholds": [] });
    assert_eq!(api_calls["policy"], default_policy);
    let api_calls_id = api_calls["id"].as_str().unwrap().to_string();
    let limit_with = |terms: Value| {
        let mut value = json!({ "type": "METERED", "limit": "100" });
        value
            .as_object_mut()
            .unwrap()
            .extend(terms.as_object().unwrap().clone());
        value
    };
    let allow_monthly = limit_with(json!({
        "overage_behavior": "ALLOW",
        "reset_period": { "type": "CALENDAR", "unit": "MONTH", "interval": 1 },
        "thresholds": [80],
    }));

    // ── A Set to row on a plan version: the terms become its override ──
    let plan_path = format!(
        "/api/v1/plan-versions/{}/entitlements",
        PLAN_VERSION_2_LEETCODE_ID.as_base62()
    );
    let created = json_or_panic(
        f.post(&plan_path)
            .json(&json!({ "entitlements": [{ "feature_id": api_calls_id, "value": allow_monthly }] }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    let row = &created["data"][0];
    assert_eq!(
        row["policy_override"],
        json!({
            "resets": "CALENDAR",
            "period": { "unit": "MONTH", "interval": 1 },
            "at_limit": "ALLOW_AND_BILL",
            "grace_pct": 0,
            "thresholds": [80],
        }),
        "{row}"
    );
    let resolved_terms = async || {
        let listed = json_or_panic(
            f.get(&plan_path).send().await.unwrap(),
            reqwest::StatusCode::OK,
        )
        .await;
        resolved_by_code(&listed, "api-calls")["value"].clone()
    };
    let resolved = resolved_terms().await;
    assert_eq!(resolved["overage_behavior"], "ALLOW", "{resolved}");
    assert_eq!(
        resolved["reset_period"],
        json!({ "type": "CALENDAR", "unit": "MONTH", "interval": 1 })
    );
    assert_eq!(resolved["thresholds"], json!([80]));

    let row_path = format!("/api/v1/entitlements/{}", row["id"].as_str().unwrap());
    let patch_ok = async |body: Value| {
        json_or_panic(
            f.patch(&row_path).json(&body).send().await.unwrap(),
            reqwest::StatusCode::OK,
        )
        .await
    };
    // Terms bringing the row back to the feature's policy remove the override.
    let back = patch_ok(json!({ "value": limit_with(json!({
        "overage_behavior": "BLOCK",
        "reset_period": { "type": "BILLING_CYCLE" },
        "thresholds": null,
    })) }))
    .await;
    assert!(back.get("policy_override").is_none(), "{back}");
    let resolved = resolved_terms().await;
    assert_eq!(resolved["overage_behavior"], "BLOCK", "{resolved}");
    assert_eq!(resolved["reset_period"]["type"], "BILLING_CYCLE");
    // An omitted term keeps the policy in effect.
    let graced = patch_ok(json!({ "value": limit_with(json!({ "grace_pct": 10 })) })).await;
    assert_eq!(
        graced["policy_override"],
        json!({ "resets": "BILLING_CYCLE", "at_limit": "DENY", "grace_pct": 10, "thresholds": [] }),
        "{graced}"
    );
    assert_eq!(resolved_terms().await["grace_pct"], 10);

    for (body, why) in [
        (
            json!({
                "value": limit_with(json!({ "overage_behavior": "ALLOW" })),
                "policy_override": default_policy,
            }),
            "terms disagreeing with the policy_override sent",
        ),
        (
            json!({ "value": limit_with(json!({ "group_by": "user_id" })) }),
            "group_by is gone",
        ),
        (
            json!({ "value": limit_with(json!({ "overage_behavior": "ALLOW", "grace_pct": 10 })) }),
            "a grace applies to DENY only",
        ),
    ] {
        assert_eq!(
            status_of(f.patch(&row_path).json(&body)).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{why}"
        );
    }
    assert_eq!(
        resolved_terms().await["grace_pct"],
        10,
        "refusals change nothing"
    );

    // ── An add-on row adds to or raises the level: it follows the feature's policy ──
    let transfer_id = f
        .create_feature(json!({
            "name": "Transfer",
            "code": "transfer",
            "kind": "LIMIT", "limit": { "counted_by": "EVENTS", "metric_id": metric },
            "product_id": platform,
        }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let add_on_id = json_or_panic(
        f.post("/api/v1/addons")
            .json(&json!({
                "name": "More transfer",
                "product_id": platform,
                "price_id": PRICE_LEETCODE_RATE_ID.as_base62(),
            }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let add_on_path = format!("/api/v1/addons/{add_on_id}/entitlements");
    let refused = f
        .post(&add_on_path)
        .json(&json!({ "entitlements": [{ "feature_id": transfer_id, "value": allow_monthly }] }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::BAD_REQUEST);
    let error: Value = refused.json().await.unwrap();
    assert!(
        error["message"].as_str().unwrap().contains("policy"),
        "{error}"
    );
    let same_terms = limit_with(json!({
        "overage_behavior": "BLOCK",
        "reset_period": { "type": "BILLING_CYCLE" },
    }));
    let accepted = json_or_panic(
        f.post(&add_on_path)
            .json(&json!({ "entitlements": [{ "feature_id": transfer_id, "value": same_terms }] }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(accepted["data"].as_array().unwrap().len(), 1);
    assert!(accepted["data"][0].get("policy_override").is_none());

    // ── Customer overrides raise by default; as a Set to, they take the terms ──
    let uber = CUST_UBER_ID.as_base62();
    let customer_path = format!("/api/v1/customers/{uber}/entitlements");
    let check = async || {
        json_or_panic(
            f.post(&format!("{customer_path}/check"))
                .json(&json!({ "feature": "api-calls", "requested": "1000" }))
                .send()
                .await
                .unwrap(),
            reqwest::StatusCode::OK,
        )
        .await
    };
    assert_eq!(
        status_of(f.post(&customer_path).json(
            &json!({ "entitlements": [{ "feature_id": api_calls_id, "value": allow_monthly }] })
        ))
        .await,
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(check().await["allowed"], false, "nothing was written");
    let set_row = json_or_panic(
        f.post(&customer_path)
            .json(&json!({ "entitlements": [{
                "feature_id": api_calls_id,
                "effect": "SET",
                "value": limit_with(json!({ "overage_behavior": "ALLOW" })),
            }] }))
            .send()
            .await
            .unwrap(),
        reqwest::StatusCode::CREATED,
    )
    .await;
    assert_eq!(
        set_row["data"][0]["policy_override"]["at_limit"], "ALLOW_AND_BILL",
        "{set_row}"
    );
    let answer = check().await;
    assert_eq!(answer["allowed"], true, "{answer}");
    assert_eq!(answer["in_overage"], true, "{answer}");
    let snapshot = json_or_panic(
        f.get(&customer_path).send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    let entry = resolved_by_code(&snapshot, "api-calls");
    assert_eq!(
        entry["levels"][0]["policy"]["at_limit"], "ALLOW_AND_BILL",
        "{entry}"
    );
    assert_eq!(
        entry["value"]["spec"]["overage_behavior"], "ALLOW",
        "{entry}"
    );
}

/// v1 created features with `feature_type` until 2.3: the shape is still accepted, mapped as the
/// entitlements_v2 migration converts features, and reads back the same.
#[tokio::test]
async fn test_rest_v1_feature_type_create_shape() {
    let f = start_rest_fixture("rest-v1-feature-type").await;
    let metric = METRIC_BANDWIDTH.as_base62();

    let metered = f
        .create_feature(json!({
            "name": "Legacy calls",
            "code": "legacy-calls",
            "feature_type": { "type": "METERED", "metric_id": metric },
            "entitlement": { "type": "METERED", "limit": "1000", "overage_behavior": "ALLOW" },
        }))
        .await;
    assert_eq!(metered["kind"], "LIMIT");
    assert_eq!(
        metered["limit"],
        json!({ "counted_by": "EVENTS", "metric_id": metric })
    );
    assert_eq!(
        metered["feature_type"],
        json!({ "type": "METERED", "metric_id": metric })
    );
    // v1 stored an omitted reset period as never resetting.
    assert_eq!(
        metered["policy"],
        json!({ "resets": "NEVER", "at_limit": "ALLOW_AND_BILL", "grace_pct": 0, "thresholds": [] })
    );
    assert_eq!(metered["entitlement"]["value"]["limit"], "1000");
    assert!(metered["entitlement"].get("policy_override").is_none());
    let read_back = json_or_panic(
        f.get("/api/v1/features/legacy-calls").send().await.unwrap(),
        reqwest::StatusCode::OK,
    )
    .await;
    assert_eq!(read_back["policy"], metered["policy"]);
    assert_eq!(read_back["feature_type"], metered["feature_type"]);

    let bare = f
        .create_feature(json!({
            "name": "Legacy bare",
            "code": "legacy-bare",
            "feature_type": { "type": "METERED", "metric_id": metric },
        }))
        .await;
    assert_eq!(
        bare["policy"],
        json!({ "resets": "BILLING_CYCLE", "at_limit": "DENY", "grace_pct": 0, "thresholds": [] }),
        "a limit without rows gets the default, as migrated"
    );

    for (code, feature_type, kind, value, read_as) in [
        (
            "legacy-flag",
            json!({ "type": "BOOLEAN" }),
            "BOOLEAN",
            Value::Null,
            json!({ "type": "BOOLEAN" }),
        ),
        (
            "legacy-config-flag",
            json!({ "type": "CONFIG", "value_type": "BOOLEAN" }),
            "BOOLEAN",
            Value::Null,
            json!({ "type": "BOOLEAN" }),
        ),
        (
            "legacy-tier",
            json!({ "type": "CONFIG", "value_type": "SELECT", "options": ["basic", "pro"] }),
            "VALUE",
            json!({ "type": "SELECT", "options": ["basic", "pro"], "ordered": false }),
            json!({ "type": "CONFIG", "value_type": "SELECT", "options": ["basic", "pro"] }),
        ),
        (
            "legacy-seats",
            json!({ "type": "CONFIG", "value_type": "NUMBER" }),
            "VALUE",
            json!({ "type": "NUMBER", "higher_is_better": true, "ordered": false }),
            json!({ "type": "CONFIG", "value_type": "NUMBER" }),
        ),
        (
            "legacy-settings",
            json!({ "type": "CONFIG", "value_type": "MAP" }),
            "VALUE",
            json!({ "type": "JSON", "ordered": false }),
            json!({ "type": "CONFIG", "value_type": "JSON" }),
        ),
    ] {
        let created = f
            .create_feature(json!({ "name": code, "code": code, "feature_type": feature_type }))
            .await;
        assert_eq!(created["kind"], kind, "{code}: {created}");
        assert_eq!(
            created.get("value").cloned().unwrap_or(Value::Null),
            value,
            "{code}"
        );
        assert_eq!(created["feature_type"], read_as, "{code}");
    }

    // `kind` stays the primary shape; with both, they must describe the same feature.
    let both = f
        .create_feature(json!({
            "name": "Both",
            "code": "both",
            "kind": "BOOLEAN",
            "feature_type": { "type": "BOOLEAN" },
        }))
        .await;
    assert_eq!(both["kind"], "BOOLEAN");
    for (body, why) in [
        (
            json!({ "name": "x", "code": "mismatch", "kind": "BOOLEAN", "feature_type": { "type": "METERED", "metric_id": metric } }),
            "kind and feature_type disagree",
        ),
        (
            json!({ "name": "x", "code": "neither" }),
            "kind or feature_type is required",
        ),
        (
            json!({ "name": "x", "code": "spec-mix", "feature_type": { "type": "BOOLEAN" }, "value": { "type": "TEXT" } }),
            "value goes with kind",
        ),
        (
            json!({ "name": "x", "code": "no-options", "feature_type": { "type": "CONFIG", "value_type": "SELECT" } }),
            "a select needs options",
        ),
        (
            json!({
                "name": "x",
                "code": "policy-mix",
                "feature_type": { "type": "METERED", "metric_id": metric },
                "policy": { "resets": "BILLING_CYCLE", "at_limit": "DENY" },
                "entitlement": { "type": "METERED", "limit": "5", "overage_behavior": "ALLOW" },
            }),
            "policy and the entitlement's terms disagree",
        ),
    ] {
        assert_eq!(
            status_of(f.post("/api/v1/features").json(&body)).await,
            reqwest::StatusCode::BAD_REQUEST,
            "{why}"
        );
    }
    let codes = [
        "mismatch",
        "neither",
        "spec-mix",
        "no-options",
        "policy-mix",
    ];
    for code in codes {
        assert_eq!(
            status_of(f.get(&format!("/api/v1/features/{code}"))).await,
            reqwest::StatusCode::NOT_FOUND,
            "{code}"
        );
    }
}
