//! The client API's public routes (`/api/client/v1/pricing-pages/{family_code}`): publishable
//! keys created, listed and revoked from the dashboard, the page read with one as API v2 reads it,
//! its allowed origins, and the key refused everywhere else.

use reqwest::StatusCode;
use reqwest::header::{
    ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, ETAG, IF_NONE_MATCH, ORIGIN, VARY,
};
use serde_json::{Value, json};
use tonic::Code;

use crate::data::ids::{CUST_UBER_ID, PLAN_LEETCODE_ID, PLAN_NOTION_ID, PRODUCT_FAMILY_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use crate::test_auth_api_key::{build_tower_svc, list_customers};
use common_domain::ids::BaseId;
use meteroid_grpc::meteroid::api::apitokens::v1::{
    ApiToken, ApiTokenKind, CreateApiTokenRequest, CreatePublishableKeyRequest,
    ListApiTokensRequest, RevokeApiTokenRequest,
};
use meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient;

struct Fixture {
    http: reqwest::Client,
    base: String,
    secret_key: String,
}

impl Fixture {
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        key: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, reqwest::header::HeaderMap, Value) {
        let request = self.http.request(method, format!("{}{path}", self.base));
        let request = match key {
            Some(key) => request.bearer_auth(key),
            None => request,
        };
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        let res = request.send().await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let text = res.text().await.unwrap();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, headers, body)
    }

    async fn server(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let (status, _, body) = self.send(method, path, Some(&self.secret_key), body).await;
        assert!(status.is_success(), "{path} {status}: {body}");
        body
    }

    async fn public_get(
        &self,
        path: &str,
        key: Option<&str>,
        headers: &[(reqwest::header::HeaderName, &str)],
    ) -> reqwest::Response {
        let mut request = self.http.get(format!("{}/api/client/v1{path}", self.base));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        for (name, value) in headers {
            request = request.header(name.clone(), *value);
        }
        request.send().await.unwrap()
    }

    /// Refused with 401, as a problem.
    async fn assert_unauthorized(&self, path: &str, key: &str) {
        let (status, _, body) = self.send(reqwest::Method::GET, path, Some(key), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        if path.starts_with("/api/v2/") || path.starts_with("/api/client/") {
            assert_eq!(body["code"], "unauthorized", "{path}: {body}");
        }
    }
}

async fn read_json(response: reqwest::Response) -> (StatusCode, reqwest::header::HeaderMap, Value) {
    let status = response.status();
    let headers = response.headers().clone();
    let text = response.text().await.unwrap();
    let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, headers, body)
}

#[tokio::test]
async fn test_publishable_keys_read_the_public_routes_only() {
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
    let mut api_tokens = clients.api_tokens.clone();

    // ── Lifecycle ────────────────────────────────────────────────────────────

    let secret = api_tokens
        .create_api_token(tonic::Request::new(CreateApiTokenRequest {
            name: "server".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    let secret_details = secret.details.unwrap();
    assert_eq!(secret_details.kind(), ApiTokenKind::Secret);
    assert_eq!(secret_details.publishable_key, None);

    let create = |name: &str, origins: Vec<&str>| CreatePublishableKeyRequest {
        name: name.to_string(),
        allowed_origins: origins.into_iter().map(str::to_string).collect(),
    };
    let open: ApiToken = api_tokens
        .create_publishable_key(tonic::Request::new(create("pricing page", vec![])))
        .await
        .unwrap()
        .into_inner()
        .details
        .unwrap();
    assert_eq!(open.kind(), ApiTokenKind::Publishable);
    let open_key = open.publishable_key.clone().expect("the key is returned");
    // The env is encoded as a secret key's is: the seeded tenant's short name.
    let env = secret_details
        .hint
        .strip_prefix("pv_")
        .and_then(|rest| rest.split('_').next())
        .unwrap()
        .to_string();
    assert!(
        open_key.starts_with(&format!("pk_{env}_")),
        "{open_key} ({env})"
    );
    assert!(
        open.hint.starts_with(&format!("pk_{env}_")),
        "{}",
        open.hint
    );
    assert!(open.allowed_origins.is_empty());

    let restricted = api_tokens
        .create_publishable_key(tonic::Request::new(create(
            "acme site",
            vec![" HTTPS://WWW.Acme.example/ ", "http://localhost:3000"],
        )))
        .await
        .unwrap()
        .into_inner()
        .details
        .unwrap();
    assert_eq!(
        restricted.allowed_origins,
        ["https://www.acme.example", "http://localhost:3000"]
    );
    let restricted_key = restricted.publishable_key.clone().unwrap();

    let invalid = api_tokens
        .create_publishable_key(tonic::Request::new(create(
            "bad",
            vec!["https://acme.example/pricing"],
        )))
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), Code::InvalidArgument, "{invalid:?}");

    // A publishable key is listed again, whole; a secret key never is.
    let listed = api_tokens
        .list_api_tokens(tonic::Request::new(ListApiTokensRequest {}))
        .await
        .unwrap()
        .into_inner()
        .api_tokens;
    let listed_open = listed.iter().find(|token| token.id == open.id).unwrap();
    assert_eq!(
        listed_open.publishable_key.as_deref(),
        Some(open_key.as_str())
    );
    let listed_secret = listed
        .iter()
        .find(|token| token.id == secret_details.id)
        .unwrap();
    assert_eq!(listed_secret.publishable_key, None);
    assert_eq!(listed_secret.kind(), ApiTokenKind::Secret);

    let fx = Fixture {
        http: http_client_tracing::client(),
        base: setup.config.rest_api_external_url.clone(),
        secret_key: secret.api_key.clone(),
    };

    // ── The page, as v2 reads it ─────────────────────────────────────────────

    // LeetCode self-serve on the ladder; Notion sales-led with its price hidden.
    fx.server(
        reqwest::Method::PATCH,
        &format!("/api/v2/plans/{}", PLAN_LEETCODE_ID.as_base62()),
        Some(json!({ "listing": { "self_serve_rank": 1, "tagline": "For teams" } })),
    )
    .await;
    fx.server(
        reqwest::Method::PATCH,
        &format!("/api/v2/plans/{}", PLAN_NOTION_ID.as_base62()),
        Some(json!({ "listing": {
            "self_serve_rank": 2,
            "sales_led": true,
            "hide_price": true,
            "contact_url": "https://acme.example/sales",
        } })),
    )
    .await;
    let leetcode = fx
        .server(
            reqwest::Method::GET,
            &format!("/api/v2/plans/{}", PLAN_LEETCODE_ID.as_base62()),
            None,
        )
        .await;

    let server_page = fx
        .server(
            reqwest::Method::GET,
            &format!("/api/v2/pricing-pages/{}", PRODUCT_FAMILY_ID.as_base62()),
            None,
        )
        .await;
    let family = server_page["family"]["code"].as_str().unwrap().to_string();
    let page_path = format!("/pricing-pages/{family}");

    let (status, headers, page) =
        read_json(fx.public_get(&page_path, Some(&open_key), &[]).await).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page, server_page);
    let cards = page["cards"].as_array().unwrap();
    let leetcode_card = cards
        .iter()
        .find(|card| card["plan"]["code"] == leetcode["code"])
        .unwrap_or_else(|| panic!("LeetCode is on the page: {page}"));
    assert_eq!(leetcode_card["tagline"], "For teams");
    for card in cards {
        if card["hide_price"] == true {
            assert!(card.get("price").is_none(), "a hidden price leaks: {card}");
        }
        assert!(card.get("plan_id").is_none(), "{card}");
    }

    // Cacheable: public for a minute, varying on the key, revalidated by its ETag.
    let cache_control = headers.get(CACHE_CONTROL).unwrap().to_str().unwrap();
    assert!(cache_control.starts_with("public"), "{cache_control}");
    let vary = headers
        .get_all(VARY)
        .iter()
        .map(|value| value.to_str().unwrap().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(",");
    assert!(vary.contains("authorization"), "{vary}");
    let etag = headers.get(ETAG).unwrap().to_str().unwrap().to_string();
    let revalidated = fx
        .public_get(
            &page_path,
            Some(&open_key),
            &[(IF_NONE_MATCH, etag.as_str())],
        )
        .await;
    assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);

    // The currency and option are the page's query, as on v2.
    let (status, _, in_currency) = read_json(
        fx.public_get(
            &format!(
                "{page_path}?currency={}",
                server_page["currency"].as_str().unwrap()
            ),
            Some(&open_key),
            &[],
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{in_currency}");
    assert_eq!(in_currency["currency"], server_page["currency"]);

    let (status, _, missing) = read_json(
        fx.public_get("/pricing-pages/no-such-family", Some(&open_key), &[])
            .await,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["code"], "not_found");

    // ── Allowed origins ──────────────────────────────────────────────────────

    let from = |origin: &'static str| [(ORIGIN, origin)];
    let allowed = fx
        .public_get(
            &page_path,
            Some(&restricted_key),
            &from("https://www.acme.example"),
        )
        .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(
        allowed
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some("https://www.acme.example")
    );
    let (status, headers, refused) = read_json(
        fx.public_get(
            &page_path,
            Some(&restricted_key),
            &from("https://evil.example"),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["code"], "forbidden");
    assert!(
        headers.get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "an unlisted origin may not read the answer"
    );
    // Not a browser's cross-site request: the origins do not apply.
    let server_side = fx.public_get(&page_path, Some(&restricted_key), &[]).await;
    assert_eq!(server_side.status(), StatusCode::OK);
    // A key without origins answers any.
    let anywhere = fx
        .public_get(&page_path, Some(&open_key), &from("https://evil.example"))
        .await;
    assert_eq!(anywhere.status(), StatusCode::OK);
    assert_eq!(
        anywhere
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some("https://evil.example")
    );

    // ── Refused everywhere else ──────────────────────────────────────────────

    // The server API, v1 and v2.
    fx.assert_unauthorized("/api/v1/customers", &open_key).await;
    fx.assert_unauthorized(&format!("/api/v2/pricing-pages/{family}"), &open_key)
        .await;
    fx.assert_unauthorized("/api/v2/customers", &open_key).await;
    // The client API's customer routes, and any route that is not public.
    for path in [
        "/api/client/v1/customer",
        "/api/client/v1/entitlements",
        "/api/client/v1/subscriptions",
        "/api/client/v1/entitlements/sso/check",
        "/api/client/v1/no-such-route",
    ] {
        fx.assert_unauthorized(path, &open_key).await;
    }
    let (status, _, body) = fx
        .send(
            reqwest::Method::POST,
            &format!("/api/client/v1{page_path}"),
            Some(&open_key),
            None,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a POST is no public read: {body}"
    );
    // The gRPC API.
    let grpc = list_customers(CustomersServiceClient::new(build_tower_svc(
        &setup.channel,
        &open_key,
    )))
    .await;
    assert_eq!(grpc.unwrap_err().code(), Code::Unauthenticated);

    // ── Only a publishable key reads the public routes ───────────────────────

    // A secret key never travels in a browser: the public routes refuse it.
    fx.assert_unauthorized(&format!("/api/client/v1{page_path}"), &secret.api_key)
        .await;
    let minted = fx
        .server(
            reqwest::Method::POST,
            &format!(
                "/api/v1/customers/{}/portal-token",
                CUST_UBER_ID.as_base62()
            ),
            Some(json!({})),
        )
        .await;
    let customer_token = minted["token"].as_str().unwrap();
    fx.assert_unauthorized(&format!("/api/client/v1{page_path}"), customer_token)
        .await;
    let (status, _, body) = read_json(fx.public_get(&page_path, None, &[]).await).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    // A forged key: a real key's id with another secret, and a malformed one.
    let id_part = open_key.rsplit_once('/').unwrap().1;
    let forged = format!("pk_{env}_{}/{id_part}", "A".repeat(28));
    fx.assert_unauthorized(&format!("/api/client/v1{page_path}"), &forged)
        .await;
    fx.assert_unauthorized(&format!("/api/client/v1{page_path}"), "pk_sand_nothing")
        .await;
    // A secret key's id under the publishable prefix is no publishable key.
    let secret_id_part = secret.api_key.rsplit_once('/').unwrap().1;
    let as_publishable = format!("pk_{env}_{}/{secret_id_part}", "A".repeat(28));
    fx.assert_unauthorized(&format!("/api/client/v1{page_path}"), &as_publishable)
        .await;

    // ── Revocation ───────────────────────────────────────────────────────────

    let doomed = api_tokens
        .create_publishable_key(tonic::Request::new(create("doomed", vec![])))
        .await
        .unwrap()
        .into_inner()
        .details
        .unwrap();
    api_tokens
        .revoke_api_token(tonic::Request::new(RevokeApiTokenRequest {
            id: doomed.id.clone(),
        }))
        .await
        .unwrap();
    let listed = api_tokens
        .list_api_tokens(tonic::Request::new(ListApiTokensRequest {}))
        .await
        .unwrap()
        .into_inner()
        .api_tokens;
    assert!(listed.iter().all(|token| token.id != doomed.id));
    fx.assert_unauthorized(
        &format!("/api/client/v1{page_path}"),
        &doomed.publishable_key.unwrap(),
    )
    .await;
}
