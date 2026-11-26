//! Browser-facing client API (`/api/client/v1`): customer portal token auth, customer
//! isolation, the shapes of API v2 (the entitlement snapshot with its `ETag`, the checks), the
//! resources and attributes a token claims, CORS and the published spec. Its public routes, read
//! with a publishable key, are `test_rest_client_public`.

use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::data::ids::{CUST_SPOTIFY_ID, CUST_UBER_ID, PLAN_LEETCODE_ID, TENANT_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, CustomerId, EntitlementEntityId, InvoiceId, SavedViewId,
};
use meteroid_grpc::meteroid::api;
use meteroid_store::domain::entitlement_fold::GrantValue;
use meteroid_store::domain::entitlements::{EntitlementNew, EntitlementSpec};
use meteroid_store::jwt_claims::{PortalJwtClaims, PortalScope, ResourceAccess};
use meteroid_store::repositories::EntitlementsInterface;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;

/// What the checks of the client and of the server answer alike; `as_of` is when each answered.
const CHECK_FIELDS: [&str; 11] = [
    "feature",
    "allowed",
    "reason",
    "value",
    "used",
    "limit",
    "remaining",
    "near_limit",
    "in_overage",
    "levels",
    "usage_source",
];

struct Fixture {
    http: reqwest::Client,
    base: String,
    api_key: String,
    jwt_secret: SecretString,
}

impl Fixture {
    async fn server_get(&self, path: &str) -> Value {
        let res = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .unwrap();
        let status = res.status();
        let body = res.text().await.unwrap();
        assert!(status.is_success(), "{path} {status}: {body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn server_post(&self, path: &str, body: Value) -> Value {
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = res.status();
        let body = res.text().await.unwrap();
        assert!(status.is_success(), "{path} {status}: {body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn server_put(&self, path: &str, body: Value) -> Value {
        let res = self
            .http
            .put(format!("{}{path}", self.base))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = res.status();
        let body = res.text().await.unwrap();
        assert!(status.is_success(), "{path} {status}: {body}");
        serde_json::from_str(&body).unwrap()
    }

    /// A client API request with the customer's token.
    fn client(&self, method: Method, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}/api/client/v1{path}", self.base))
            .bearer_auth(token)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> (StatusCode, HeaderMap, Value) {
        let res = request.send().await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.text().await.unwrap();
        let body = serde_json::from_str(&body).unwrap_or(Value::String(body));
        (status, headers, body)
    }

    async fn client_post(
        &self,
        path: &str,
        token: &str,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        self.send(self.client(Method::POST, path, token).json(&body))
            .await
    }

    async fn mint(&self, customer: CustomerId, body: Value) -> String {
        let minted = self
            .server_post(
                &format!("/api/v1/customers/{}/portal-token", customer.as_base62()),
                body,
            )
            .await;
        minted["token"].as_str().unwrap().to_string()
    }

    fn hand_mint(&self, claims: PortalJwtClaims) -> String {
        claims.encode(&self.jwt_secret).unwrap()
    }

    async fn client_get(&self, path: &str, token: Option<&str>) -> (StatusCode, Value) {
        let request = self.http.get(format!("{}/api/client/v1{path}", self.base));
        let request = match token {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        let res = request.send().await.unwrap();
        let status = res.status();
        let body = res.text().await.unwrap();
        let body = serde_json::from_str(&body).unwrap_or(Value::String(body));
        (status, body)
    }

    async fn client_ok(&self, path: &str, token: &str) -> Value {
        let (status, body) = self.client_get(path, Some(token)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        body
    }

    /// Refused on every customer route, as a problem carrying the request id.
    async fn assert_rejected(&self, token: Option<&str>, status: StatusCode, code: &str) {
        for path in [
            "/customer",
            "/entitlements",
            "/entitlements/sso/check",
            "/subscriptions",
        ] {
            let (actual, body) = self.client_get(path, token).await;
            assert_eq!(actual, status, "{path}: {body}");
            assert_eq!(body["code"], code, "{path}: {body}");
            assert_eq!(body["status"], status.as_u16(), "{path}: {body}");
            assert!(body["request_id"].is_string(), "{path}: {body}");
        }
    }
}

fn customer_claims(
    customer: CustomerId,
    scopes: Option<Vec<PortalScope>>,
    expires_in: Option<chrono::Duration>,
) -> PortalJwtClaims {
    PortalJwtClaims::new(
        TENANT_ID,
        ResourceAccess::Customer(customer),
        scopes,
        expires_in,
    )
    .unwrap()
}

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

#[tokio::test]
async fn test_rest_client_api() {
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
                name: "client-api-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key;

    let fx = Fixture {
        http: http_client_tracing::client(),
        base: setup.config.rest_api_external_url.clone(),
        api_key,
        jwt_secret: setup.config.jwt_secret.clone(),
    };

    // Uber gets SSO through its LeetCode subscription, the plan granting it; Spotify has neither.
    fx.server_post(
        "/api/v2/features",
        json!({ "code": "sso", "name": "SSO", "type": { "type": "boolean" } }),
    )
    .await;
    let sso = setup
        .store
        .get_feature_by_id_or_code(AliasOr::Alias("sso".to_owned()), TENANT_ID)
        .await
        .unwrap();
    setup
        .store
        .create_entitlement(
            &Actor::System,
            EntitlementNew {
                tenant_id: TENANT_ID,
                entity: EntitlementEntityId::Plan(PLAN_LEETCODE_ID),
                resource_id: None,
                spec: EntitlementSpec::new(sso.id, GrantValue::Boolean(true)),
            },
        )
        .await
        .unwrap();
    let subscription = fx
        .server_post(
            "/api/v1/subscriptions",
            json!({
                "plan_id": PLAN_LEETCODE_ID.as_base62(),
                "customer_id_or_alias": CUST_UBER_ID.as_base62(),
                "start_date": chrono::Utc::now().date_naive(),
                "activation_condition": "ON_START",
                "charge_automatically": false,
            }),
        )
        .await;
    let subscription_id = subscription["id"].as_str().unwrap();

    let uber_token = fx
        .mint(
            CUST_UBER_ID,
            json!({ "scopes": ["read"], "expires_in_seconds": 3600 }),
        )
        .await;
    let spotify_token = fx.mint(CUST_SPOTIFY_ID, json!({})).await;

    // Customer: who it is, as API v2 reads the customer, nothing the merchant keeps to itself.
    let customer = fx.client_ok("/customer", &uber_token).await;
    let server_customer = fx
        .server_get(&format!("/api/v2/customers/{}", CUST_UBER_ID.as_base62()))
        .await;
    for field in keys(&customer) {
        assert!(
            [
                "alias",
                "currency",
                "email",
                "first_name",
                "id",
                "last_name",
                "name",
                "type"
            ]
            .contains(&field),
            "{field}"
        );
        assert_eq!(customer[field], server_customer[field], "{field}");
    }
    assert_eq!(customer["id"], CUST_UBER_ID.as_base62());
    assert_eq!(customer["name"], "Uber");
    assert_eq!(customer["type"], "company");

    // Entitlements: the server API's snapshot of the customer, unchanged, with its etag.
    let uber = CUST_UBER_ID.as_base62();
    let (status, headers, entitlements) = fx
        .send(fx.client(Method::GET, "/entitlements", &uber_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{entitlements}");
    let server_entitlements = fx
        .server_get(&format!("/api/v2/customers/{uber}/entitlements"))
        .await;
    assert_eq!(entitlements, server_entitlements);
    let etag = entitlements["etag"].as_str().unwrap();
    assert_eq!(
        headers.get("etag").and_then(|v| v.to_str().ok()),
        Some(format!("\"{etag}\"").as_str())
    );
    let sso_entitlement = entitlements["entitlements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["feature"]["code"] == "sso")
        .expect("the subscribed plan grants sso");
    assert_eq!(sso_entitlement["value"], json!({ "type": "on" }));
    let (status, _, unchanged) = fx
        .send(
            fx.client(Method::GET, "/entitlements", &uber_token)
                .header("if-none-match", format!("\"{etag}\"")),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED, "{unchanged}");

    // Checks, GET and POST: the token's customer, answered as the server API answers it.
    let server_check = fx
        .server_get(&format!("/api/v2/customers/{uber}/entitlements/sso/check"))
        .await;
    assert_eq!(server_check["allowed"], true, "{server_check}");
    let (status, headers, by_query) = fx
        .send(fx.client(Method::GET, "/entitlements/sso/check", &uber_token))
        .await;
    assert_eq!(status, StatusCode::OK, "{by_query}");
    assert!(headers.get("deprecation").is_none());
    let (status, headers, by_body) = fx
        .client_post(
            "/entitlements/check",
            &uber_token,
            json!({ "feature": "sso" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{by_body}");
    assert!(headers.get("deprecation").is_none());
    for key in CHECK_FIELDS {
        assert_eq!(by_query[key], server_check[key], "GET {key}");
        assert_eq!(by_body[key], server_check[key], "POST {key}");
    }
    assert_eq!(by_query["feature"]["code"], "sso");
    assert_eq!(by_query["value"], json!({ "type": "on" }));
    assert!(by_query.get("reason").is_none(), "{by_query}");
    let (status, refused) = fx
        .client_get("/entitlements/no-such-feature/check", Some(&uber_token))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
    assert_eq!(refused["code"], "not_found", "{refused}");
    let spotify_check = fx
        .client_ok("/entitlements/sso/check", &spotify_token)
        .await;
    assert_eq!(spotify_check["allowed"], false, "{spotify_check}");
    assert_eq!(spotify_check["reason"], "not_entitled", "{spotify_check}");

    // `explain` and `current_count` take a secret key.
    for path in [
        "/entitlements/sso/check?explain=true",
        "/entitlements/sso/check?current_count=1",
    ] {
        let (status, body) = fx.client_get(path, Some(&uber_token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert_eq!(body["code"], "forbidden", "{path}: {body}");
    }
    for body in [
        json!({ "feature": "sso", "explain": true }),
        json!({ "feature": "sso", "current_count": "1" }),
    ] {
        let (status, _, refused) = fx
            .client_post("/entitlements/check", &uber_token, body.clone())
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}: {refused}");
        assert_eq!(refused["code"], "forbidden", "{body}: {refused}");
    }

    // The client API takes API v2's inputs: v1's, and the inputs no check takes any more, are
    // refused.
    for (query, parameter) in [
        ("group[region]=eu", "group"),
        ("group_by=region", "group_by"),
        ("as_of=2024-01-01T00:00:00Z", "as_of"),
        ("subscription_id=sub_1", "subscription_id"),
        ("resources[site_id]=s1", "resources"),
    ] {
        let path = format!("/entitlements/sso/check?{query}");
        let (status, body) = fx.client_get(&path, Some(&uber_token)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path}: {body}");
        assert_eq!(body["errors"][0]["path"], parameter, "{path}: {body}");
    }
    let (status, body) = fx
        .client_get("/entitlements?resources[site_id]=s1", Some(&uber_token))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["errors"][0]["path"], "resources", "{body}");
    for body in [
        json!({ "feature": "sso", "subscription_id": subscription_id }),
        json!({ "feature": "sso", "resources": { "site_id": "s1" } }),
    ] {
        let (status, _, refused) = fx
            .client_post("/entitlements/check", &uber_token, body.clone())
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused}");
    }
    let (status, _, unknown) = fx
        .client_post(
            "/entitlements/check",
            &uber_token,
            json!({ "feature": "sso", "group": { "region": "eu" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{unknown}");

    // A token names only what it claims: its resource and attributes, run whole when the request
    // names none; without claims, the customer only.
    fx.server_post(
        "/api/v2/resource-types",
        json!({
            "code": "site",
            "name": "Site",
            "plural_name": "Sites",
            "keys": ["site_id"],
            "attributes": { "environment": ["production", "staging"] },
        }),
    )
    .await;
    let s1 = fx
        .server_put(
            &format!("/api/v2/customers/{uber}/resources/site/s1"),
            json!({}),
        )
        .await;
    let claimed = fx
        .server_post(
            &format!("/api/v2/customers/{uber}/portal-token"),
            json!({
                "resource": { "type": "site", "key": "s1" },
                "attributes": { "site.environment": "staging" },
            }),
        )
        .await["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let on_s1 = fx.client_ok("/entitlements", &claimed).await;
    assert_eq!(
        on_s1["resource"],
        json!({ "type": "site", "key": "s1", "id": s1["id"] }),
        "{on_s1}"
    );
    let named = fx
        .client_ok(
            "/entitlements?resource[type]=site&resource[key]=s1",
            &claimed,
        )
        .await;
    assert_eq!(named["resource"], on_s1["resource"], "{named}");
    let s1_id = s1["id"].as_str().unwrap();
    let by_id = fx
        .client_ok(&format!("/entitlements?resource={s1_id}"), &claimed)
        .await;
    assert_eq!(by_id["resource"], on_s1["resource"], "{by_id}");
    fx.client_ok(
        "/entitlements/sso/check?resource[type]=site&resource[key]=s1",
        &claimed,
    )
    .await;
    let (status, _, body) = fx
        .client_post(
            "/entitlements/check",
            &claimed,
            json!({ "feature": "sso", "resource": { "type": "site", "key": "s1" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for path in [
        "/entitlements?resource[type]=site&resource[key]=s2",
        "/entitlements?attributes[site.environment]=production",
        "/entitlements/sso/check?resource[type]=site&resource[key]=s2",
    ] {
        let (status, body) = fx.client_get(path, Some(&claimed)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert_eq!(body["code"], "forbidden", "{path}: {body}");
    }
    for path in [
        "/entitlements?resource[type]=site&resource[key]=s1",
        "/entitlements/sso/check?resource[type]=site&resource[key]=s1",
    ] {
        let (status, body) = fx.client_get(path, Some(&uber_token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
    }
    let (status, _, body) = fx
        .client_post(
            "/entitlements/check",
            &uber_token,
            json!({ "feature": "sso", "resource": { "type": "site", "key": "s1" } }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Subscriptions: API v2's summary of each, less what only the merchant reads.
    let subscriptions = fx.client_ok("/subscriptions", &uber_token).await;
    assert_eq!(subscriptions["has_more"], false, "{subscriptions}");
    let listed = subscriptions["data"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{subscriptions}");
    let listed = &listed[0];
    let mut listed_keys = keys(listed);
    listed_keys.retain(|key| *key != "trial");
    assert_eq!(
        listed_keys,
        [
            "currency",
            "current_period",
            "dates",
            "id",
            "period",
            "plan",
            "status",
        ]
    );
    let summary = fx
        .server_get(&format!("/api/v2/subscriptions/{subscription_id}/summary"))
        .await;
    for field in keys(listed) {
        if field == "plan" {
            continue;
        }
        assert_eq!(listed[field], summary[field], "{field}");
    }
    for field in ["code", "version", "option"] {
        assert_eq!(
            listed["plan"][field], summary["plan"][field],
            "plan.{field}"
        );
    }
    assert_eq!(listed["id"], subscription_id);
    assert_eq!(listed["plan"]["name"], "LeetCode");
    assert_eq!(listed["period"], "monthly");
    for internal in [
        "mrr",
        "invoicing",
        "custom_properties",
        "resource_id",
        "resource",
        "customer",
    ] {
        assert!(listed.get(internal).is_none(), "{internal}: {listed}");
    }

    // Another customer's token only ever sees that customer, whatever the query says.
    let spotify = fx
        .client_ok(
            &format!("/customer?customer_id={}", CUST_UBER_ID.as_base62()),
            &spotify_token,
        )
        .await;
    assert_eq!(spotify["id"], CUST_SPOTIFY_ID.as_base62());
    let spotify_subscriptions = fx.client_ok("/subscriptions", &spotify_token).await;
    assert_eq!(
        spotify_subscriptions,
        json!({ "data": [], "next_cursor": null, "has_more": false })
    );
    let spotify_entitlements = fx.client_ok("/entitlements", &spotify_token).await;
    assert!(
        spotify_entitlements["entitlements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["feature"]["code"] == "sso")
            .all(|e| e.get("value").is_none()),
        "{spotify_entitlements}"
    );

    // `manage` implies `read`, and legacy tokens (no scopes, no aud) keep full access.
    let manage_only = fx.hand_mint(customer_claims(
        CUST_UBER_ID,
        Some(vec![PortalScope::Manage]),
        Some(chrono::Duration::hours(1)),
    ));
    fx.client_ok("/customer", &manage_only).await;
    let mut legacy = customer_claims(CUST_UBER_ID, None, Some(chrono::Duration::hours(1)));
    legacy.scopes = None;
    legacy.aud = None;
    let legacy = fx.hand_mint(legacy);
    assert_eq!(fx.client_ok("/customer", &legacy).await, customer);

    // Expired: TOKEN_EXPIRED only when the signature is valid.
    let expired_claims = customer_claims(
        CUST_UBER_ID,
        Some(vec![PortalScope::Read]),
        Some(chrono::Duration::minutes(-1)),
    );
    let expired = fx.hand_mint(expired_claims.clone());
    fx.assert_rejected(Some(&expired), StatusCode::UNAUTHORIZED, "token_expired")
        .await;
    let forged_expired = expired_claims
        .encode(&SecretString::new("not-the-secret".into()))
        .unwrap();
    fx.assert_rejected(
        Some(&forged_expired),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;

    // Stored tokens: expired, for another resource, or unknown.
    let stored_expired = setup
        .store
        .customer_portal_token(
            TENANT_ID,
            CUST_UBER_ID,
            chrono::Duration::minutes(-1),
            Some(vec![PortalScope::Read]),
            false,
            Default::default(),
        )
        .await
        .unwrap()
        .token;
    fx.assert_rejected(
        Some(&stored_expired),
        StatusCode::UNAUTHORIZED,
        "token_expired",
    )
    .await;
    let dashboard_token = setup
        .store
        .dashboard_portal_token(TENANT_ID, SavedViewId::new())
        .await
        .unwrap();
    fx.assert_rejected(Some(&dashboard_token), StatusCode::FORBIDDEN, "forbidden")
        .await;
    fx.assert_rejected(
        Some(&"A".repeat(32)),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;

    // Tokens without `exp` are refused, whatever their resource.
    let never_expires = fx.hand_mint(customer_claims(CUST_UBER_ID, None, None));
    fx.assert_rejected(
        Some(&never_expires),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;
    let invoice_token = fx.hand_mint(
        PortalJwtClaims::new(
            TENANT_ID,
            ResourceAccess::Invoice(InvoiceId::new()),
            None,
            None,
        )
        .unwrap(),
    );
    fx.assert_rejected(
        Some(&invoice_token),
        StatusCode::UNAUTHORIZED,
        "unauthorized",
    )
    .await;

    // A valid token for another resource, or without a read scope, is forbidden.
    let expiring_invoice_token = fx.hand_mint(
        PortalJwtClaims::new(
            TENANT_ID,
            ResourceAccess::Invoice(InvoiceId::new()),
            None,
            Some(chrono::Duration::hours(1)),
        )
        .unwrap(),
    );
    fx.assert_rejected(
        Some(&expiring_invoice_token),
        StatusCode::FORBIDDEN,
        "forbidden",
    )
    .await;
    let no_scopes = fx.hand_mint(customer_claims(
        CUST_UBER_ID,
        Some(vec![]),
        Some(chrono::Duration::hours(1)),
    ));
    fx.assert_rejected(Some(&no_scopes), StatusCode::FORBIDDEN, "forbidden")
        .await;

    // Tenant API keys, dashboard sessions and garbage never reach the client API.
    fx.assert_rejected(Some(&fx.api_key), StatusCode::UNAUTHORIZED, "unauthorized")
        .await;
    fx.assert_rejected(Some(&auth.token), StatusCode::UNAUTHORIZED, "unauthorized")
        .await;
    fx.assert_rejected(Some("not-a-jwt"), StatusCode::UNAUTHORIZED, "unauthorized")
        .await;
    fx.assert_rejected(None, StatusCode::UNAUTHORIZED, "unauthorized")
        .await;

    // A portal token is not an API key on the server API.
    let res = fx
        .http
        .get(format!(
            "{}/api/v1/customers/{}",
            fx.base,
            CUST_UBER_ID.as_base62()
        ))
        .bearer_auth(&uber_token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // CORS: the SDK's version header is allowed and rate-limit headers are readable.
    let preflight = fx
        .http
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/api/client/v1/entitlements", fx.base),
        )
        .header("origin", "https://app.acme.example")
        .header("access-control-request-method", "GET")
        .header(
            "access-control-request-headers",
            "authorization,meteroid-version",
        )
        .send()
        .await
        .unwrap();
    assert!(preflight.status().is_success(), "{}", preflight.status());
    let allowed = preflight
        .headers()
        .get("access-control-allow-headers")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(allowed.contains("meteroid-version"), "{allowed}");

    let res = fx
        .http
        .get(format!("{}/api/client/v1/entitlements", fx.base))
        .bearer_auth(&uber_token)
        .header("origin", "https://app.acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://app.acme.example")
    );
    let exposed = res
        .headers()
        .get("access-control-expose-headers")
        .unwrap()
        .to_str()
        .unwrap();
    for header in [
        "retry-after",
        "ratelimit-limit",
        "ratelimit-remaining",
        "ratelimit-reset",
        "request-id",
        "meteroid-version",
    ] {
        assert!(exposed.contains(header), "{header} not in {exposed}");
    }

    // Spec: the client API is published on its own, and only there. Its entitlement check is core's
    // (ADR 0011 §3.14).
    let client_spec = fx.server_get("/api-docs/client/openapi.json").await;
    let mut client_paths = keys(&client_spec["paths"]);
    client_paths.sort_unstable();
    assert_eq!(
        client_paths,
        [
            "/api/client/v1/customer",
            "/api/client/v1/entitlements",
            "/api/client/v1/entitlements/check",
            "/api/client/v1/entitlements/{feature}/check",
            "/api/client/v1/pricing-pages/{family_code}",
            "/api/client/v1/subscriptions",
        ]
    );
    assert!(client_spec["components"]["securitySchemes"]["customer_token"].is_object());
    assert!(client_spec["components"]["securitySchemes"]["publishable_key"].is_object());
    assert!(client_spec["components"]["schemas"]["Problem"].is_object());
    let server_spec = fx.server_get("/api-docs/openapi.json").await;
    assert!(
        keys(&server_spec["paths"])
            .iter()
            .all(|path| !path.starts_with("/api/client/"))
    );
}
