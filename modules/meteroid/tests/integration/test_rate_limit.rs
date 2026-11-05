use crate::data::ids::{CUST_UBER_ID, PLAN_VERSION_PAID_TRIAL_ID, TENANT_ID};
use crate::{helpers, meteroid_it};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CheckoutSessionId, CustomerId};
use meteroid::config::{GrpcRateLimitConfig, RestRateLimitConfig};
use meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenRequest;
use meteroid_grpc::meteroid::api::customers::v1::ListCustomerRequest;
use meteroid_grpc::meteroid::portal::checkout::v1::GetCheckoutRequest;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_client::PortalCheckoutServiceClient;
use meteroid_it::container::SeedLevel;
use meteroid_rest_common::error::{ErrorCode, RestErrorResponse};
use meteroid_store::domain::CreateCheckoutSession;
use meteroid_store::domain::checkout_sessions::CheckoutType;
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::jwt_claims::{PortalJwtClaims, ResourceAccess};
use meteroid_store::repositories::checkout_sessions::CheckoutSessionsInterface;
use meteroid_store::repositories::coupons::CouponInterface;
use rust_decimal::Decimal;

#[tokio::test]
async fn test_rate_limit() {
    helpers::init::logging();

    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;

    // Start a real Redis instance so the rate limiter is active.
    let redis_container = meteroid_it::container::start_redis_container().await;
    let redis_port = redis_container.get_host_port_ipv4(6379).await.unwrap();
    let redis_url = format!("redis://127.0.0.1:{}", redis_port);

    let fred_client = meteroid::process::connect_redis(&meteroid::config::RedisConfig {
        url: Some(redis_url),
        password: None,
    })
    .await
    .expect("Redis container should be reachable")
    .expect("REDIS_URL is set");

    let rate_limiter = meteroid::process::rate_limiter(Some(&fred_client)).await;
    assert!(rate_limiter.is_enabled(), "Rate limiter should be enabled");

    let setup = meteroid_it::container::start_meteroid_with_rate_limiter(
        postgres_connection_string,
        SeedLevel::MINIMAL,
        rate_limiter,
        meteroid::config::RateLimitConfig {
            rest: RestRateLimitConfig {
                prod_rps: 2,
                non_prod_rps: 2,
                prod_metering_rps: 2,
                non_prod_metering_rps: 2,
                prod_metering_eps: 100,
                non_prod_metering_eps: 100,
                client_prod_rps: 2,
                client_non_prod_rps: 2,
                prod_usage_costs_rpm: 100,
                non_prod_usage_costs_rpm: 100,
            },
            grpc: GrpcRateLimitConfig { rpm: 2 },
        },
    )
    .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );

    // ── REST rate limit ──────────────────────────────────────────────────────

    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(CreateApiTokenRequest {
            name: "rate-limit-test-key".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .api_key;

    let http_client = http_client_tracing::client();
    let url = format!("{}/api/v1/customers", setup.config.rest_api_external_url);

    let mut rest_rate_limited = None;
    for _ in 0..10 {
        let resp = http_client
            .get(&url)
            .bearer_auth(&api_key)
            .send()
            .await
            .expect("HTTP request failed");

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            rest_rate_limited = Some(resp);
            break;
        }
    }

    let resp =
        rest_rate_limited.expect("Expected a 429 Too Many Requests response within 10 requests");

    let headers = resp.headers();
    assert_eq!(
        headers.get("RateLimit-Limit").and_then(|v| v.to_str().ok()),
        Some("2"),
        "RateLimit-Limit header should match the configured capacity"
    );
    assert_eq!(
        headers
            .get("RateLimit-Remaining")
            .and_then(|v| v.to_str().ok()),
        Some("0"),
        "RateLimit-Remaining should be 0 when rate limited"
    );
    assert!(
        headers.contains_key("RateLimit-Reset"),
        "RateLimit-Reset header must be present"
    );

    let retry_after_secs = headers
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .expect("Retry-After header must be a valid integer");

    let body = resp
        .json::<RestErrorResponse>()
        .await
        .expect("Failed to deserialize body");

    assert_eq!(body.code, ErrorCode::TooManyRequests);
    assert!(
        body.message.contains("Rate limit exceeded"),
        "Response body message should describe the rate limit: {}",
        body.message
    );

    // ── REST recovery after Retry-After seconds ──────────────────────────────

    tokio::time::sleep(std::time::Duration::from_secs(retry_after_secs)).await;

    let recovery_resp = http_client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .expect("HTTP request failed after rate limit window");

    assert_eq!(
        recovery_resp.status(),
        reqwest::StatusCode::OK,
        "Request should succeed after waiting for the rate limit window to reset"
    );

    // ── Client API rate limit, per customer ─────────────────────────────────

    let client_token = |customer: CustomerId| {
        PortalJwtClaims::new(
            TENANT_ID,
            ResourceAccess::Customer(customer),
            None,
            Some(chrono::Duration::hours(1)),
        )
        .unwrap()
        .encode(&setup.config.jwt_secret)
        .unwrap()
    };
    let client_url = format!(
        "{}/api/client/v1/subscriptions",
        setup.config.rest_api_external_url
    );
    let limited_token = client_token(CustomerId::new());

    let mut client_rate_limited = None;
    for _ in 0..10 {
        let resp = http_client
            .get(&client_url)
            .bearer_auth(&limited_token)
            .header("origin", "https://app.acme.example")
            .send()
            .await
            .expect("HTTP request failed");

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            client_rate_limited = Some(resp);
            break;
        }
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    let resp = client_rate_limited
        .expect("Expected a 429 from the client API within 10 requests of one customer");
    let headers = resp.headers();
    assert!(headers.contains_key("Retry-After"));
    assert_eq!(
        headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let exposed = headers
        .get("access-control-expose-headers")
        .and_then(|v| v.to_str().ok())
        .expect("CORS exposes headers on a 429");
    assert!(exposed.contains("retry-after"), "{exposed}");
    let body = resp
        .json::<RestErrorResponse>()
        .await
        .expect("Failed to deserialize body");
    assert_eq!(body.code, ErrorCode::TooManyRequests);

    let other_customer = http_client
        .get(&client_url)
        .bearer_auth(client_token(CustomerId::new()))
        .send()
        .await
        .expect("HTTP request failed");
    assert_eq!(
        other_customer.status(),
        reqwest::StatusCode::OK,
        "Each customer has its own bucket"
    );

    // ── gRPC rate limit ──────────────────────────────────────────────────────

    let mut grpc_rate_limited = None;
    for _ in 0..10 {
        let result = clients
            .customers
            .clone()
            .list_customers(tonic::Request::new(ListCustomerRequest {
                search: None,
                archived: None,
                pagination: None,
                order_by: Some("name.asc".to_string()),
                lifecycle_state: vec![],
                country: vec![],
                filters: vec![],
            }))
            .await;

        if let Err(status) = result
            && status.code() == tonic::Code::ResourceExhausted
        {
            grpc_rate_limited = Some(status);
            break;
        }
    }

    let status =
        grpc_rate_limited.expect("Expected gRPC ResourceExhausted status within 10 requests");
    assert_eq!(status.code(), tonic::Code::ResourceExhausted);

    // A checkout link has no actor: it is limited by its session.
    let mut checkout = PortalCheckoutServiceClient::new(setup.channel.clone());
    let link = checkout_token(&setup, CheckoutSessionId::new());
    let mut checkout_rate_limited = false;
    for _ in 0..10 {
        let status = get_checkout(&mut checkout, &link, None)
            .await
            .expect_err("no such session");
        if status.code() == tonic::Code::ResourceExhausted {
            checkout_rate_limited = true;
            break;
        }
    }
    assert!(
        checkout_rate_limited,
        "a checkout link is rate limited within 10 requests"
    );
}

fn checkout_token(
    setup: &meteroid_it::container::MeteroidSetup,
    session: CheckoutSessionId,
) -> String {
    PortalJwtClaims::new(
        TENANT_ID,
        ResourceAccess::CheckoutSession(session),
        None,
        Some(chrono::Duration::hours(1)),
    )
    .unwrap()
    .encode(&setup.config.jwt_secret)
    .unwrap()
}

async fn get_checkout(
    client: &mut PortalCheckoutServiceClient<tonic::transport::Channel>,
    token: &str,
    coupon_code: Option<&str>,
) -> Result<(), tonic::Status> {
    let mut request = tonic::Request::new(GetCheckoutRequest {
        coupon_code: coupon_code.map(str::to_owned),
    });
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().unwrap());
    client.get_checkout(request).await.map(|_| ())
}

/// Codes typed at checkout draw on the customer's promo-code budget: a usable code is free, an
/// unknown one costs a try, and the budget runs out across all of the customer's checkouts.
#[tokio::test]
async fn checkout_promo_codes_draw_on_the_customer_code_budget() {
    helpers::init::logging();
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let redis_container = meteroid_it::container::start_redis_container().await;
    let redis_port = redis_container.get_host_port_ipv4(6379).await.unwrap();
    let fred_client = meteroid::process::connect_redis(&meteroid::config::RedisConfig {
        url: Some(format!("redis://127.0.0.1:{redis_port}")),
        password: None,
    })
    .await
    .expect("Redis container should be reachable")
    .expect("REDIS_URL is set");
    let rate_limiter = meteroid::process::rate_limiter(Some(&fred_client)).await;
    let setup = meteroid_it::container::start_meteroid_with_rate_limiter(
        database,
        SeedLevel::PLANS,
        rate_limiter,
        meteroid::config::RateLimitConfig {
            rest: RestRateLimitConfig::default(),
            grpc: GrpcRateLimitConfig { rpm: 1_000 },
        },
    )
    .await;
    crate::data::payment::run_mock_payment_provider_seed(&setup.store.pool, false).await;
    crate::data::payment::run_customer_payment_methods_seed(&setup.store.pool).await;
    setup
        .store
        .create_coupon(
            Actor::System,
            CouponNew {
                code: "SAVE10".to_owned(),
                description: "Ten percent off".to_owned(),
                tenant_id: TENANT_ID,
                discount: CouponDiscount::Percentage(Decimal::from(10)),
                expires_at: None,
                redemption_limit: None,
                recurring_value: None,
                reusable: true,
                plan_ids: vec![],
            },
        )
        .await
        .expect("coupon");

    let mut client = PortalCheckoutServiceClient::new(setup.channel.clone());
    let mut links = Vec::new();
    for _ in 0..2 {
        let session = setup
            .store
            .create_checkout_session(CreateCheckoutSession {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                plan_version_id: PLAN_VERSION_PAID_TRIAL_ID,
                billing_start_date: None,
                billing_day_anchor: None,
                net_terms: None,
                trial_duration_days: None,
                end_date: None,
                auto_advance_invoices: true,
                charge_automatically: true,
                invoice_memo: None,
                invoice_threshold: None,
                purchase_order: None,
                payment_methods_config: None,
                components: None,
                add_ons: None,
                coupon_code: None,
                coupon_ids: vec![],
                expires_in_hours: Some(24),
                metadata: None,
                checkout_type: CheckoutType::SelfServe,
                subscription_id: None,
                change_date: None,
                success_url: None,
                cancel_url: None,
            })
            .await
            .expect("checkout session");
        links.push(checkout_token(&setup, session.id));
    }

    for _ in 0..8 {
        get_checkout(&mut client, &links[0], Some("SAVE10"))
            .await
            .expect("a usable code costs nothing");
    }
    for (attempt, link) in [&links[0], &links[0], &links[0], &links[1], &links[1]]
        .into_iter()
        .enumerate()
    {
        let refused = get_checkout(&mut client, link, Some(&format!("GUESS{attempt}")))
            .await
            .expect_err("an unknown code");
        assert_ne!(
            refused.code(),
            tonic::Code::ResourceExhausted,
            "try {attempt}"
        );
    }
    let exhausted = get_checkout(&mut client, &links[1], Some("GUESS5"))
        .await
        .expect_err("the budget is spent");
    assert_eq!(exhausted.code(), tonic::Code::ResourceExhausted);
    assert!(
        exhausted.message().contains("Too many codes tried"),
        "{exhausted:?}"
    );
    get_checkout(&mut client, &links[1], None)
        .await
        .expect("a checkout without a code is unaffected");
}

#[tokio::test]
async fn test_usage_cost_reads_share_a_per_minute_quota() {
    helpers::init::logging();

    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let redis_container = meteroid_it::container::start_redis_container().await;
    let redis_port = redis_container.get_host_port_ipv4(6379).await.unwrap();
    let fred_client = meteroid::process::connect_redis(&meteroid::config::RedisConfig {
        url: Some(format!("redis://127.0.0.1:{redis_port}")),
        password: None,
    })
    .await
    .expect("Redis container should be reachable")
    .expect("REDIS_URL is set");
    let rate_limiter = meteroid::process::rate_limiter(Some(&fred_client)).await;

    let setup = meteroid_it::container::start_meteroid_with_rate_limiter(
        postgres_connection_string,
        SeedLevel::MINIMAL,
        rate_limiter,
        meteroid::config::RateLimitConfig {
            rest: RestRateLimitConfig {
                prod_usage_costs_rpm: 3,
                non_prod_usage_costs_rpm: 3,
                ..RestRateLimitConfig::default()
            },
            grpc: GrpcRateLimitConfig { rpm: 100 },
        },
    )
    .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let api_key = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    )
    .api_tokens
    .clone()
    .create_api_token(tonic::Request::new(CreateApiTokenRequest {
        name: "usage-costs-quota".to_string(),
    }))
    .await
    .unwrap()
    .into_inner()
    .api_key;

    let http_client = http_client_tracing::client();
    let base = &setup.config.rest_api_external_url;
    let subscription = common_domain::ids::SubscriptionId::new().as_base62();
    let get = |url: String| {
        let request = http_client.get(url).bearer_auth(&api_key);
        async move { request.send().await.expect("HTTP request failed").status() }
    };

    for _ in 0..2 {
        let status = get(format!("{base}/api/v1/usage/subscription/{subscription}")).await;
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    }
    let status = get(format!(
        "{base}/api/v1/usage/customer/{}/costs?metric=bandwidth&granularity=DAY&from=2026-01-01T00:00:00Z&to=2026-01-02T00:00:00Z",
        CUST_UBER_ID.as_base62()
    ))
    .await;
    assert_ne!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "the third read is allowed"
    );

    let status = get(format!("{base}/api/v1/usage/subscription/{subscription}")).await;
    assert_eq!(status, reqwest::StatusCode::TOO_MANY_REQUESTS);

    let status = get(format!("{base}/api/v1/customers")).await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "other routes keep their own limit"
    );
}
