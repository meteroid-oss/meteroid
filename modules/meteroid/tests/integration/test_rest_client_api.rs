//! Browser-facing client API (`/api/client/v1`): customer portal token auth, customer
//! isolation, CORS and the published spec.

use reqwest::StatusCode;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::data::ids::{
    CUST_SPOTIFY_ID, CUST_UBER_ID, PLAN_LEETCODE_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID,
};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, SavedViewId};
use meteroid_grpc::meteroid::api;
use meteroid_store::jwt_claims::{PortalJwtClaims, PortalScope, ResourceAccess};
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;

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

    async fn assert_rejected(&self, token: Option<&str>, status: StatusCode, code: &str) {
        for path in ["/customer", "/entitlements", "/subscriptions"] {
            let (actual, body) = self.client_get(path, token).await;
            assert_eq!(actual, status, "{path}: {body}");
            assert_eq!(body["code"], code, "{path}: {body}");
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

    // Uber gets an SSO entitlement through a LeetCode subscription; Spotify has neither.
    let sso = fx
        .server_post(
            "/api/v1/features",
            json!({ "name": "SSO", "code": "sso", "feature_type": { "type": "BOOLEAN" } }),
        )
        .await;
    fx.server_post(
        &format!(
            "/api/v1/plan-versions/{}/entitlements",
            PLAN_VERSION_1_LEETCODE_ID.as_base62()
        ),
        json!({ "entitlements": [{
            "feature_id": sso["id"],
            "value": { "type": "BOOLEAN", "enabled": true },
        }] }),
    )
    .await;
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

    // Customer: a minimal projection of the server resource.
    let customer = fx.client_ok("/customer", &uber_token).await;
    assert_eq!(
        keys(&customer),
        ["alias", "billing_email", "currency", "id", "name"]
    );
    let server_customer = fx
        .server_get(&format!("/api/v1/customers/{}", CUST_UBER_ID.as_base62()))
        .await;
    for field in keys(&customer) {
        assert_eq!(customer[field], server_customer[field], "{field}");
    }
    assert_eq!(customer["id"], CUST_UBER_ID.as_base62());
    assert_eq!(customer["name"], "Uber");

    // Entitlements: the server API's payload, unchanged.
    let entitlements = fx.client_ok("/entitlements", &uber_token).await;
    let server_entitlements = fx
        .server_get(&format!(
            "/api/v1/customers/{}/entitlements",
            CUST_UBER_ID.as_base62()
        ))
        .await;
    assert_eq!(entitlements, server_entitlements);
    let sso_entitlement = entitlements["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["feature"]["code"] == "sso")
        .expect("the subscribed plan grants sso");
    assert_eq!(
        sso_entitlement["value"],
        json!({ "type": "BOOLEAN", "enabled": true })
    );

    // Subscriptions: a minimal projection of the server resource.
    let subscriptions = fx.client_ok("/subscriptions", &uber_token).await;
    let listed = subscriptions["data"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{subscriptions}");
    let listed = &listed[0];
    assert_eq!(
        keys(listed),
        [
            "billing_period",
            "currency",
            "current_period_end",
            "current_period_start",
            "end_date",
            "id",
            "plan_id",
            "plan_name",
            "plan_version",
            "start_date",
            "status",
        ]
    );
    let server_subscription = fx
        .server_get(&format!("/api/v1/subscriptions/{subscription_id}"))
        .await;
    for field in keys(listed) {
        let server_field = if field == "billing_period" {
            "period"
        } else {
            field
        };
        assert_eq!(listed[field], server_subscription[server_field], "{field}");
    }
    assert_eq!(listed["id"], subscription_id);
    assert_eq!(listed["plan_name"], "LeetCode");
    assert_eq!(listed["billing_period"], "MONTHLY");

    // Another customer's token only ever sees that customer, whatever the query says.
    let spotify = fx
        .client_ok(
            &format!("/customer?customer_id={}", CUST_UBER_ID.as_base62()),
            &spotify_token,
        )
        .await;
    assert_eq!(spotify["id"], CUST_SPOTIFY_ID.as_base62());
    let spotify_subscriptions = fx.client_ok("/subscriptions", &spotify_token).await;
    assert_eq!(spotify_subscriptions, json!({ "data": [] }));
    let spotify_entitlements = fx.client_ok("/entitlements", &spotify_token).await;
    assert!(
        !spotify_entitlements.to_string().contains("\"sso\""),
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
    fx.assert_rejected(Some(&expired), StatusCode::UNAUTHORIZED, "TOKEN_EXPIRED")
        .await;
    let forged_expired = expired_claims
        .encode(&SecretString::new("not-the-secret".into()))
        .unwrap();
    fx.assert_rejected(
        Some(&forged_expired),
        StatusCode::UNAUTHORIZED,
        "UNAUTHORIZED",
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
        )
        .await
        .unwrap()
        .token;
    fx.assert_rejected(
        Some(&stored_expired),
        StatusCode::UNAUTHORIZED,
        "TOKEN_EXPIRED",
    )
    .await;
    let dashboard_token = setup
        .store
        .dashboard_portal_token(TENANT_ID, SavedViewId::new())
        .await
        .unwrap();
    fx.assert_rejected(Some(&dashboard_token), StatusCode::FORBIDDEN, "FORBIDDEN")
        .await;
    fx.assert_rejected(
        Some(&"A".repeat(32)),
        StatusCode::UNAUTHORIZED,
        "UNAUTHORIZED",
    )
    .await;

    // Tokens without `exp` are refused, whatever their resource.
    let never_expires = fx.hand_mint(customer_claims(CUST_UBER_ID, None, None));
    fx.assert_rejected(
        Some(&never_expires),
        StatusCode::UNAUTHORIZED,
        "UNAUTHORIZED",
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
        "UNAUTHORIZED",
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
        "FORBIDDEN",
    )
    .await;
    let no_scopes = fx.hand_mint(customer_claims(
        CUST_UBER_ID,
        Some(vec![]),
        Some(chrono::Duration::hours(1)),
    ));
    fx.assert_rejected(Some(&no_scopes), StatusCode::FORBIDDEN, "FORBIDDEN")
        .await;

    // Tenant API keys, dashboard sessions and garbage never reach the client API.
    fx.assert_rejected(Some(&fx.api_key), StatusCode::UNAUTHORIZED, "UNAUTHORIZED")
        .await;
    fx.assert_rejected(Some(&auth.token), StatusCode::UNAUTHORIZED, "UNAUTHORIZED")
        .await;
    fx.assert_rejected(Some("not-a-jwt"), StatusCode::UNAUTHORIZED, "UNAUTHORIZED")
        .await;
    fx.assert_rejected(None, StatusCode::UNAUTHORIZED, "UNAUTHORIZED")
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
    ] {
        assert!(exposed.contains(header), "{header} not in {exposed}");
    }

    // Spec: the client API is published on its own, and only there. Its entitlement check is the
    // usage controls module's.
    let client_spec = fx.server_get("/api-docs/client/openapi.json").await;
    let mut client_paths = keys(&client_spec["paths"]);
    client_paths.sort_unstable();
    assert_eq!(
        client_paths,
        [
            "/api/client/v1/customer",
            "/api/client/v1/entitlements",
            "/api/client/v1/subscriptions",
        ]
    );
    assert!(client_spec["components"]["securitySchemes"]["customer_token"].is_object());
    let server_spec = fx.server_get("/api-docs/openapi.json").await;
    assert!(
        keys(&server_spec["paths"])
            .iter()
            .all(|path| !path.starts_with("/api/client/"))
    );
}
