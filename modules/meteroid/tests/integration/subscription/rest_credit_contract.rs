//! The public REST contract of credit: credit-unit prices survive a GET → PUT round trip, a
//! subscription exposes its unit pack, delegated and express credentials never reach the customer
//! money routes, and promotional-credit codes follow the coupon code rules.
use std::str::FromStr;
use std::sync::Arc;

use chrono::{NaiveDate, NaiveTime};
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, ConnectedAccountId, PlanVersionId, SubscriptionId};
use meteroid_store::clients::usage::{
    GroupedUsageData, MockUsageDataParams, MutableMockUsageClient, UsageData,
};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::enums::{InvoiceStatusEnum, TenantEnvironmentEnum};
use meteroid_store::domain::{ApiTokenNew, OrganizationNew, UsagePeriod};
use meteroid_store::errors::StoreError;
use meteroid_store::jwt_claims::PortalJwtClaims;
use meteroid_store::repositories::OrganizationsInterface;
use meteroid_store::repositories::api_tokens::ApiTokensInterface;
use meteroid_store::repositories::coupons::CouponInterface;

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

struct Rest {
    base: String,
    client: reqwest::Client,
    key: String,
}

impl Rest {
    async fn new(env: &TestEnv) -> Self {
        Self::with_key(env, api_key(env, TENANT_ID).await)
    }

    fn with_key(env: &TestEnv, key: String) -> Self {
        Self {
            base: env.setup.config.rest_api_external_url.clone(),
            client: http_client_tracing::client(),
            key,
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.key)
            .header("Idempotency-Key", uuid::Uuid::now_v7().to_string());
        if let Some(body) = body {
            request = request.json(&body);
        }
        read(request.send().await.unwrap()).await
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::PUT, path, Some(body)).await
    }
}

async fn read(response: reqwest::Response) -> (StatusCode, Value) {
    let status = response.status();
    let body = response.text().await.unwrap();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

async fn api_key(env: &TestEnv, tenant_id: common_domain::ids::TenantId) -> String {
    env.store()
        .insert_api_token(
            Actor::System,
            ApiTokenNew {
                name: "credit contract".to_owned(),
                tenant_id,
            },
        )
        .await
        .expect("api key")
        .0
}

fn decimal(value: &Value) -> Decimal {
    match value {
        Value::String(text) => Decimal::from_str(text).expect("a decimal string"),
        other => Decimal::from_str(&other.to_string()).expect("a decimal number"),
    }
}

async fn create_credit_unit(rest: &Rest, code: &str) -> String {
    let (status, unit) = rest
        .post(
            "/api/v1/credit-units",
            json!({
                "code": code,
                "name": "Halyard credit",
                "plural_name": "Halyard credits",
                "precision": 0
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{unit}");
    unit["id"].as_str().unwrap().to_owned()
}

async fn create_plan(rest: &Rest, name: &str, components: Value) -> Value {
    let (status, plan) = rest
        .post(
            "/api/v1/plans",
            json!({
                "name": name,
                "product_family_id": PRODUCT_FAMILY_ID.as_base62(),
                "plan_type": "STANDARD",
                "status": "ACTIVE",
                "currency": "EUR",
                "components": components,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");
    plan
}

/// What a generated SDK sends back after reading the plan: its components as inputs.
fn replacement_of(plan: &Value) -> Value {
    let components = plan["price_components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| {
            json!({
                "name": component["name"],
                "product_id": component["product_id"],
                "fee": component["fee"],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "name": plan["name"],
        "status": "ACTIVE",
        "currency": "EUR",
        "components": components,
    })
}

async fn subscribe(
    env: &TestEnv,
    customer: common_domain::ids::CustomerId,
    plan: &Value,
) -> SubscriptionId {
    subscription()
        .customer(customer)
        .plan_version(PlanVersionId::from_str(plan["version_id"].as_str().unwrap()).unwrap())
        .start_date(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

/// Studio's overage, 1 credit per unit at EUR 0.01 a credit: a GET → PUT keeps the unit and its
/// rate, the same PUT without `credit_unit` is refused, and 480 uncovered credits bill EUR 4.80.
#[tokio::test]
async fn a_rest_round_trip_keeps_credit_pricing_and_bills_overage_at_its_rate() {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    env.set_consolidate_and_grace(false, 720).await;
    let rest = Rest::new(&env).await;
    let unit_id = create_credit_unit(&rest, "HALYARD").await;

    let created = create_plan(
        &rest,
        "Studio",
        json!([{
            "name": "Tokens",
            "fee": {
                "type": "USAGE",
                "metric_id": METRIC_BANDWIDTH.as_base62(),
                "pricing": { "type": "PER_UNIT", "rate": "1" },
                "cadence": "MONTHLY",
                "credit_unit": { "credit_unit_id": unit_id, "rate": "0.01" }
            }
        }]),
    )
    .await;
    let plan_path = format!("/api/v1/plans/{}", created["id"].as_str().unwrap());
    let (status, read_back) = rest.get(&plan_path).await;
    assert_eq!(status, StatusCode::OK, "{read_back}");
    let unit = &read_back["price_components"][0]["fee"]["credit_unit"];
    assert_eq!(unit["credit_unit_id"], unit_id.as_str(), "{read_back}");
    assert_eq!(decimal(&unit["rate"]), Decimal::new(1, 2));

    let mut dropped = replacement_of(&read_back);
    dropped["components"][0]["fee"]
        .as_object_mut()
        .unwrap()
        .remove("credit_unit");
    let (status, refused) = rest.put(&plan_path, dropped).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("'Tokens' is priced in Halyard credits")),
        "{refused}"
    );
    let (_, unchanged) = rest.get(&plan_path).await;
    assert_eq!(unchanged["version_id"], created["version_id"]);

    let (status, replaced) = rest.put(&plan_path, replacement_of(&read_back)).await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert_ne!(replaced["version_id"], created["version_id"]);
    let unit = &replaced["price_components"][0]["fee"]["credit_unit"];
    assert_eq!(unit["credit_unit_id"], unit_id.as_str(), "{replaced}");
    assert_eq!(decimal(&unit["rate"]), Decimal::new(1, 2));

    let subscription_id = subscribe(&env, CUST_UBER_ID, &replaced).await;
    env.process_cycle_transitions().await;
    let start = NaiveDate::from_ymd_opt(2024, 1, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    let end = NaiveDate::from_ymd_opt(2024, 2, 1)
        .unwrap()
        .and_time(NaiveTime::MIN);
    usage.set_usage(
        MockUsageDataParams {
            metric_id: METRIC_BANDWIDTH,
            period_start: start,
            period_end: end,
        },
        UsageData {
            data: vec![GroupedUsageData {
                value: Decimal::from(480),
                dimensions: std::collections::HashMap::new(),
            }],
            period: UsagePeriod { start, end },
        },
    );
    env.process_due_events().await;

    let invoices = env.get_invoices(subscription_id).await;
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0];
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    let line = invoice
        .line_items
        .iter()
        .find(|line| line.credit_unit.is_some())
        .expect("a credit-priced line");
    let credit = line.credit_unit.as_ref().unwrap();
    assert_eq!((credit.consumed, credit.covered), (480, 0));
    assert_eq!(
        line.amount_subtotal, 480,
        "480 credits at EUR 0.01, not EUR 480"
    );
}

/// A subscription's credit pack in a credit unit reads with its unit and its rollover, and its
/// credit-priced usage with the unit's id.
#[tokio::test]
async fn a_rest_subscription_exposes_its_unit_pack_and_rollover() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    let unit_id = create_credit_unit(&rest, "HALYARD").await;
    let plan = create_plan(
        &rest,
        "Studio",
        json!([
            {
                "name": "Studio credits",
                "fee": {
                    "type": "CREDIT",
                    "unit_price": "0",
                    "quantity": 1,
                    "credit_amount": "5000",
                    "cadence": "MONTHLY",
                    "expiry": { "unit": "BILLING_PERIOD", "value": 1 },
                    "rollover": { "max_amount": "1000", "expiry": { "unit": "MONTH", "value": 1 } },
                    "credit_unit_id": unit_id
                }
            },
            {
                "name": "Tokens",
                "fee": {
                    "type": "USAGE",
                    "metric_id": METRIC_BANDWIDTH.as_base62(),
                    "pricing": { "type": "PER_UNIT", "rate": "1" },
                    "cadence": "MONTHLY",
                    "credit_unit": { "credit_unit_id": unit_id, "rate": "0.01" }
                }
            }
        ]),
    )
    .await;
    let subscription_id = subscribe(&env, CUST_SPOTIFY_ID, &plan).await;

    let (status, body) = rest
        .get(&format!(
            "/api/v1/subscriptions/{}",
            subscription_id.as_base62()
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fees = body["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component["fee"].clone())
        .collect::<Vec<_>>();
    let pack = fees
        .iter()
        .find(|fee| fee["type"] == "CREDIT")
        .expect("the pack");
    assert_eq!(
        pack["credit_unit"],
        json!({ "credit_unit_id": unit_id, "code": "HALYARD", "precision": 0 }),
        "{body}"
    );
    assert_eq!(
        decimal(&pack["rollover"]["max_amount"]),
        Decimal::from(1000)
    );
    assert_eq!(
        pack["rollover"]["expiry"],
        json!({ "unit": "MONTH", "value": 1 })
    );
    let usage = fees
        .iter()
        .find(|fee| fee["type"] == "USAGE")
        .expect("the usage");
    assert_eq!(
        usage["credit_unit"]["credit_unit_id"],
        unit_id.as_str(),
        "{body}"
    );
    assert_eq!(usage["credit_unit"]["code"], "HALYARD");
    assert_eq!(decimal(&usage["credit_unit"]["rate"]), Decimal::new(1, 2));
}

/// A platform acting for a connected account and express organizations get 403 on top-ups,
/// credit grants, wallet writes and budgets; express reads stay reachable. Connect tokens are
/// covered by the Enterprise `connect` module.
#[tokio::test]
async fn delegated_and_express_credentials_cannot_reach_customer_money_routes() {
    let env = test_env_with_seed(SeedLevel::MINIMAL).await;
    let customer = format!("/api/v1/customers/{}", CUST_UBER_ID.as_base62());
    let denied = [
        (
            reqwest::Method::POST,
            format!("{customer}/top-ups"),
            Some(json!({ "amount": 1000 })),
        ),
        (
            reqwest::Method::GET,
            format!("{customer}/credit-grants"),
            None,
        ),
        (
            reqwest::Method::POST,
            format!("{customer}/credit-grants"),
            Some(json!({ "amount": 1000, "type": "PROMOTIONAL" })),
        ),
        (
            reqwest::Method::PATCH,
            format!("{customer}/wallets/EUR"),
            Some(json!({ "usage_at_zero": "ALWAYS_PAUSE" })),
        ),
        (reqwest::Method::GET, format!("{customer}/budgets"), None),
    ];

    let platform_key = api_key(&env, TENANT_ID).await;
    let account = ConnectedAccountId::new().as_base62();
    let as_account = |method: reqwest::Method, path: &str| {
        http_client_tracing::client()
            .request(
                method,
                format!("{}{path}", env.setup.config.rest_api_external_url),
            )
            .bearer_auth(&platform_key)
            .header("x-meteroid-connect-account", &account)
    };
    for (method, path, body) in &denied {
        let mut request = as_account(method.clone(), path);
        if let Some(body) = body {
            request = request.json(body);
        }
        let (status, refused) = read(request.send().await.unwrap()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Connect account {method} {path}: {refused}"
        );
    }
    let (status, _) = read(
        as_account(reqwest::Method::GET, &format!("{customer}/wallets"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "without the Connect module, acting for a connected account is refused"
    );

    let express = env
        .store()
        .insert_express_organization(
            OrganizationNew {
                trade_name: "Express Co".to_owned(),
                country: CountryCode::from_str("FR").unwrap(),
                invoicing_entity: None,
            },
            USER_ID,
            TenantEnvironmentEnum::Production,
        )
        .await
        .expect("express organization");
    let express_key = api_key(&env, express.tenants[0].id).await;
    let express = Rest::with_key(&env, express_key.clone());
    for (method, path, body) in &denied {
        let (status, refused) = express.send(method.clone(), path, body.clone()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "express {method} {path}: {refused}"
        );
    }
    for path in [
        format!("{customer}/wallets"),
        format!("{customer}/credit-ledger"),
    ] {
        let (status, body) = express.get(&path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "express GET {path}: {body}");
    }

    // A portal session opened with these credentials inherits the deny; a direct key's does not.
    let delegated = |body: &Value| {
        PortalJwtClaims::decode(
            body["token"].as_str().expect("a portal token"),
            &env.setup.config.jwt_secret,
        )
        .expect("a valid portal token")
        .delegated
    };
    for (rest, delegated_session) in [(&express, true), (&Rest::new(&env).await, false)] {
        let (status, created) = rest
            .post(
                "/api/v1/customers",
                json!({ "name": "Portal customer", "currency": "EUR" }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let portal_customer = created["id"].as_str().expect("a customer id");
        let (status, token) = rest
            .post(
                &format!("/api/v1/customers/{portal_customer}/portal-token"),
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{token}");
        assert_eq!(delegated(&token), delegated_session, "{token}");
    }

    // The gRPC mint inherits it too, and the session it opens cannot top up.
    let (_, created) = express
        .post(
            "/api/v1/customers",
            json!({ "name": "Portal customer", "currency": "EUR" }),
        )
        .await;
    let mut customers = meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient::new(env.setup.channel.clone());
    let mut request = tonic::Request::new(
        meteroid_grpc::meteroid::api::customers::v1::GenerateCustomerPortalTokenRequest {
            customer_id: created["id"].as_str().expect("a customer id").to_owned(),
        },
    );
    request
        .metadata_mut()
        .insert("x-api-key", express_key.parse().unwrap());
    let token = customers
        .generate_customer_portal_token(request)
        .await
        .expect("an express key mints a portal token")
        .into_inner()
        .token;
    assert!(delegated(&json!({ "token": token })));
    let mut wallets = meteroid_grpc::meteroid::portal::wallets::v1::portal_wallets_service_client::PortalWalletsServiceClient::new(env.setup.channel.clone());
    let mut request = tonic::Request::new(
        meteroid_grpc::meteroid::portal::wallets::v1::TopUpWalletRequest {
            wallet_id: "EUR".to_owned(),
            amount: 1_000,
            payment_method_id: "pm".to_owned(),
            idempotency_key: "delegated".to_owned(),
            expected_quote: None,
            return_url: None,
        },
    );
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().unwrap());
    let denied = wallets
        .top_up_wallet(request)
        .await
        .expect_err("a delegated session cannot top up");
    assert_eq!(denied.code(), tonic::Code::PermissionDenied, "{denied:?}");
}

fn credit_coupon(code: &str) -> CouponNew {
    CouponNew {
        code: code.to_owned(),
        description: "Welcome credit".to_owned(),
        tenant_id: TENANT_ID,
        discount: CouponDiscount::Credit {
            currency: "EUR".to_owned(),
            amount: Decimal::from(25),
            expires_in_days: Some(30),
            scope: Default::default(),
        },
        expires_at: None,
        redemption_limit: None,
        recurring_value: None,
        reusable: false,
        plan_ids: vec![],
    }
}

/// Promotional-credit codes are refused reusable, over 64 characters or padded with spaces, are
/// stored upper-case and redeemed whatever the case, like discount codes.
#[tokio::test]
async fn promotional_credit_codes_are_validated_and_case_insensitive() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    let credit = |code: &str, reusable: bool| {
        json!({
            "code": code,
            "discount": { "type": "CREDIT", "currency": "EUR", "amount": "25.00" },
            "reusable": reusable,
        })
    };
    let (status, refused) = rest.post("/api/v1/coupons", credit("WELCOME", true)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, refused) = rest
        .post("/api/v1/coupons", credit(&"W".repeat(65), false))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    for code in [" PADDED", "PADDED ", &"X".repeat(65)] {
        let refused = env
            .store()
            .create_coupon(Actor::System, credit_coupon(code))
            .await
            .expect_err("an invalid code is refused");
        assert!(
            matches!(refused.current_context(), StoreError::InvalidArgument(_)),
            "{code:?}"
        );
    }
    let mut discount = credit_coupon(" SPACED");
    discount.discount = CouponDiscount::Percentage(Decimal::from(10));
    assert!(
        env.store()
            .create_coupon(Actor::System, discount)
            .await
            .is_err(),
        "a discount code with surrounding spaces is refused too"
    );

    let (status, created) = rest
        .post("/api/v1/coupons", credit("welcome-25", false))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["code"], "WELCOME-25");

    let mut shadow = credit_coupon("Welcome-25");
    shadow.discount = CouponDiscount::Percentage(Decimal::from(10));
    let shadowing = env
        .store()
        .create_coupon(Actor::System, shadow)
        .await
        .expect_err("a discount code cannot shadow a credit code");
    assert!(matches!(
        shadowing.current_context(),
        StoreError::DuplicateValue { .. }
    ));

    let mut exact = credit_coupon("Summer10");
    exact.discount = CouponDiscount::Percentage(Decimal::from(10));
    env.store()
        .create_coupon(Actor::System, exact)
        .await
        .expect("a discount code keeps its case");
    let store = env.store();
    let found = |codes: Vec<String>| async move {
        store
            .list_coupons_by_codes(TENANT_ID, &codes)
            .await
            .unwrap()
            .into_iter()
            .map(|coupon| coupon.code)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        found(vec!["summer10".to_owned()]).await,
        vec!["Summer10"],
        "every code matches ignoring the case of ASCII letters"
    );
    assert_eq!(found(vec!["Summer10".to_owned()]).await, vec!["Summer10"]);
    assert_eq!(
        found(vec!["Welcome-25".to_owned()]).await,
        vec!["WELCOME-25"]
    );

    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let redeemed = env
        .services()
        .redeem_coupon_credit_code(TENANT_ID, CUST_UBER_ID, "  welcome-25 ", None)
        .await
        .expect("redeemed whatever the case");
    assert_eq!(redeemed.amount_minor, 2_500);
}

/// `POST /subscriptions` matches coupon codes whatever their case, an exact code first, and
/// refuses the request, naming them, when a code names no live coupon.
#[tokio::test]
async fn rest_subscription_coupon_codes_ignore_case_and_refuse_unknown_codes() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    for (code, percent) in [("Summer10", 10), ("SUMMER10", 20)] {
        let mut coupon = credit_coupon(code);
        coupon.discount = CouponDiscount::Percentage(Decimal::from(percent));
        coupon.reusable = true;
        env.store()
            .create_coupon(Actor::System, coupon)
            .await
            .expect("two discount codes differing in case");
    }
    for code in ["Straße", "été-10"] {
        let mut coupon = credit_coupon(code);
        coupon.discount = CouponDiscount::Percentage(Decimal::from(1));
        coupon.reusable = true;
        env.store()
            .create_coupon(Actor::System, coupon)
            .await
            .expect("a non-ASCII discount code");
    }
    let mut welcome = credit_coupon("Welcome5");
    welcome.discount = CouponDiscount::Percentage(Decimal::from(5));
    welcome.reusable = true;
    env.store()
        .create_coupon(Actor::System, welcome)
        .await
        .expect("a discount code");
    let create = |codes: Vec<&str>| {
        json!({
            "plan_id": PLAN_LEETCODE_ID.as_base62(),
            "customer_id_or_alias": CUST_UBER_ID.as_base62(),
            "start_date": chrono::Utc::now().date_naive(),
            "activation_condition": "ON_START",
            "charge_automatically": false,
            "coupon_codes": codes,
        })
    };

    let (status, refused) = rest
        .post(
            "/api/v1/subscriptions",
            create(vec!["welcome5", "NO-SUCH", "summer10"]),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let message = refused["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("unknown coupon codes: NO-SUCH"),
        "{refused}"
    );
    assert!(
        message.contains("summer10"),
        "ambiguous ignoring case: {refused}"
    );
    assert!(!message.contains("welcome5"), "{refused}");

    let (status, refused) = rest
        .post("/api/v1/subscriptions", create(vec!["ÉTÉ-10"]))
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "only ASCII letters fold: {refused}"
    );

    let (status, created) = rest
        .post(
            "/api/v1/subscriptions",
            create(vec!["welcome5", "SUMMER10", "Straße", "STRAßE", "été-10"]),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let mut applied: Vec<&str> = created["applied_coupons"]
        .as_array()
        .expect("applied coupons")
        .iter()
        .map(|applied| applied["coupon"]["code"].as_str().unwrap())
        .collect();
    applied.sort_unstable();
    assert_eq!(applied, vec!["SUMMER10", "Straße", "Welcome5", "été-10"]);
}

/// A code a release that did not validate codes stored with surrounding spaces is matched by the
/// code itself, at `POST /subscriptions` as at every other lookup.
#[tokio::test]
async fn a_coupon_code_stored_with_surrounding_spaces_is_matched_by_the_code() {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::coupon::dsl as c;

    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    let mut coupon = credit_coupon("SAVE10");
    coupon.discount = CouponDiscount::Percentage(Decimal::from(10));
    coupon.reusable = true;
    let save = env
        .store()
        .create_coupon(Actor::System, coupon)
        .await
        .expect("a discount code");
    diesel::update(c::coupon.filter(c::id.eq(save.id)))
        .set(c::code.eq(" SAVE10 "))
        .execute(&mut env.conn().await)
        .await
        .expect("stored padded, as before codes were validated");

    let found: Vec<_> = env
        .store()
        .list_coupons_by_codes(TENANT_ID, &["SAVE10".to_owned(), "save10".to_owned()])
        .await
        .expect("lookup")
        .into_iter()
        .map(|coupon| coupon.id)
        .collect();
    assert_eq!(found, vec![save.id]);

    let (status, created) = rest
        .post(
            "/api/v1/subscriptions",
            json!({
                "plan_id": PLAN_LEETCODE_ID.as_base62(),
                "customer_id_or_alias": CUST_UBER_ID.as_base62(),
                "start_date": chrono::Utc::now().date_naive(),
                "activation_condition": "ON_START",
                "charge_automatically": false,
                "coupon_codes": ["SAVE10"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["applied_coupons"][0]["coupon"]["code"], " SAVE10 ",
        "{created}"
    );
}

async fn webhook_events(env: &TestEnv) -> Vec<meteroid_store::domain::outbox_event::OutboxEvent> {
    use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
    use meteroid_store::domain::pgmq::PgmqQueue;
    use meteroid_store::repositories::pgmq::PgmqInterface;
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("webhook queue")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value(message.0).ok())
        .collect()
}

fn funded_payload(
    events: &[meteroid_store::domain::outbox_event::OutboxEvent],
    grant_id: &str,
) -> Value {
    use meteroid::api_rest::webhooks::out_model::WebhookOutWalletFundedEventData;
    use meteroid_store::domain::outbox_event::OutboxEvent;
    let funded = events
        .iter()
        .find_map(|event| match event {
            OutboxEvent::WalletFunded(funded) if funded.credit_grant_id.as_base62() == grant_id => {
                Some(funded.as_ref().clone())
            }
            _ => None,
        })
        .expect("wallet.funded for the grant");
    serde_json::to_value(WebhookOutWalletFundedEventData::from(funded)).unwrap()
}

async fn cancel_by_hand(env: &TestEnv, subscription_id: SubscriptionId) {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use diesel_models::enums::SubscriptionStatusEnum;
    use diesel_models::schema::subscription::dsl as s;
    diesel::update(s::subscription.filter(s::id.eq(subscription_id)))
        .set(s::status.eq(SubscriptionStatusEnum::Cancelled))
        .execute(&mut env.store().get_conn().await.unwrap())
        .await
        .expect("cancel the subscription");
}

/// T7: a grant made through the API for one subscription's usage of one product reads back with
/// exactly that, is listed by subscription, and its `wallet.funded` names both; a subscription of
/// another customer, an ended one or one billed in another currency is refused, and a grant
/// without a subscription pays any charge of any subscription, as before.
#[tokio::test]
async fn a_rest_grant_belongs_to_the_subscription_it_names() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    let a = subscription()
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let b = subscription()
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let product = PRODUCT_PLATFORM_FEE_ID.as_base62();
    let grants = format!(
        "/api/v1/customers/{}/credit-grants",
        CUST_UBER_ID.as_base62()
    );

    let (status, grant) = rest
        .post(
            &grants,
            json!({
                "type": "PROMOTIONAL", "amount": 5_000, "currency": "EUR",
                "subscription_id": a.as_base62(), "applies_to": "USAGE",
                "applicable_product_ids": [product],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    assert_eq!(grant["subscription_id"], a.as_base62());
    assert_eq!(grant["applies_to"], "USAGE");
    assert_eq!(grant["applicable_product_ids"], json!([product]));
    assert_eq!(grant["ended_at"], Value::Null);
    assert_eq!(grant["kind"], "BONUS");
    assert_eq!(grant["kind_label"], "Free credit");
    let id = grant["id"].as_str().unwrap().to_owned();
    let listed = |body: &Value| {
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|grant| grant["id"] == id.as_str())
    };

    let (status, of_a) = rest
        .get(&format!("{grants}?subscription_id={}", a.as_base62()))
        .await;
    assert_eq!(status, StatusCode::OK, "{of_a}");
    assert!(listed(&of_a), "{of_a}");
    let (status, usable_by_b) = rest
        .get(&format!(
            "{grants}?usable_by_subscription_id={}",
            b.as_base62()
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{usable_by_b}");
    assert!(!listed(&usable_by_b), "{usable_by_b}");
    let (status, both) = rest
        .get(&format!(
            "{grants}?subscription_id={}&usable_by_subscription_id={}",
            a.as_base62(),
            b.as_base62()
        ))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{both}");

    let funded = funded_payload(&webhook_events(&env).await, &id);
    assert_eq!(funded["subscription_id"], a.as_base62());
    assert_eq!(funded["applicable_product_ids"], json!([product]));
    assert_eq!(funded["applies_to"], "USAGE");

    let (_, wallets) = rest
        .get(&format!(
            "/api/v1/customers/{}/wallets",
            CUST_UBER_ID.as_base62()
        ))
        .await;
    let eur = &wallets["data"][0];
    assert_eq!(eur["currency"], "EUR", "{wallets}");
    assert_eq!(eur["credit_on_account_usage_only"], 5_000);
    assert_eq!(
        eur["subscription_credit"],
        json!([{ "subscription_id": a.as_base62(), "credit_on_account": 5_000 }])
    );

    let others = subscription()
        .customer(CUST_SPOTIFY_ID)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let ended = subscription()
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    cancel_by_hand(&env, ended).await;
    for (subscription_id, currency) in [(others, "EUR"), (ended, "EUR"), (a, "USD")] {
        let (status, refused) = rest
            .post(
                &grants,
                json!({
                    "type": "PROMOTIONAL", "amount": 1_000, "currency": currency,
                    "subscription_id": subscription_id.as_base62(),
                }),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{subscription_id}: {refused}"
        );
    }
    let (_, wallets) = rest
        .get(&format!(
            "/api/v1/customers/{}/wallets",
            CUST_UBER_ID.as_base62()
        ))
        .await;
    assert!(
        wallets["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|wallet| wallet["currency"] != "USD"),
        "a refused grant opens no wallet: {wallets}"
    );

    let (status, anywhere) = rest
        .post(&grants, json!({ "type": "PROMOTIONAL", "amount": 2_000 }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{anywhere}");
    assert_eq!(anywhere["subscription_id"], Value::Null);
    assert_eq!(anywhere["applies_to"], "ALL");
    assert_eq!(anywhere["kind"], "ACCOUNT_BALANCE");
    let (_, usable_by_b) = rest
        .get(&format!(
            "{grants}?usable_by_subscription_id={}",
            b.as_base62()
        ))
        .await;
    assert!(
        usable_by_b["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|grant| grant["id"] == anywhere["id"]),
        "customer-wide credit is usable by every subscription: {usable_by_b}"
    );
}

/// T3, API part: once its subscription ended, a grant's unspent credit lapses as
/// `credit_grant.expired` with reason SUBSCRIPTION_ENDED and its subscription, and the grant reads
/// EXPIRED with its `ended_at`.
#[tokio::test]
async fn credit_ending_with_its_subscription_is_reported_as_such() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    let a = subscription()
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let grants = format!(
        "/api/v1/customers/{}/credit-grants",
        CUST_UBER_ID.as_base62()
    );
    let (status, grant) = rest
        .post(
            &grants,
            json!({
                "type": "PROMOTIONAL", "amount": 10_000, "subscription_id": a.as_base62(),
                "applies_to": "USAGE",
                "expires_at": (chrono::Utc::now() + chrono::Duration::days(20)).to_rfc3339(),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{grant}");
    let id = grant["id"].as_str().unwrap().to_owned();
    let grant_id = common_domain::ids::CreditGrantId::from_proto(&id).unwrap();

    // What the subscription end hook records: the grant ended 13 hours ago, past the seeded
    // entity's 12-hour grace.
    {
        use diesel_async::SimpleAsyncConnection;
        env.store()
            .get_conn()
            .await
            .unwrap()
            .batch_execute(&format!(
                "SET session_replication_role = replica;
                 UPDATE customer_credit_grant SET effective_at = now() - interval '15 hours',
                     ends_at = now() - interval '13 hours', end_reason = 'SUBSCRIPTION_ENDED'
                 WHERE id = '{}';
                 SET session_replication_role = origin;",
                grant_id.as_uuid()
            ))
            .await
            .expect("end the grant with its subscription");
    }
    use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
    assert_eq!(env.store().expire_customer_credit(200).await.unwrap(), 1);

    use meteroid::api_rest::webhooks::out_model::WebhookOutCreditGrantExpiredEventData;
    use meteroid_store::domain::outbox_event::OutboxEvent;
    let expired = webhook_events(&env)
        .await
        .into_iter()
        .find_map(|event| match event {
            OutboxEvent::CreditGrantExpired(expired) if expired.credit_grant_id == grant_id => {
                Some(serde_json::to_value(
                    WebhookOutCreditGrantExpiredEventData::from(*expired),
                ))
            }
            _ => None,
        })
        .expect("credit_grant.expired")
        .unwrap();
    assert_eq!(expired["reason"], "SUBSCRIPTION_ENDED");
    assert_eq!(expired["subscription_id"], a.as_base62());
    assert_eq!(expired["expired_amount"], 10_000);
    assert_eq!(expired["applies_to"], "USAGE");

    let (_, of_a) = rest
        .get(&format!("{grants}?subscription_id={}", a.as_base62()))
        .await;
    let read = of_a["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|grant| grant["id"] == id.as_str())
        .cloned()
        .expect("the ended grant");
    assert_eq!(read["status"], "EXPIRED");
    assert!(read["ended_at"].is_string(), "{read}");
    assert_eq!(read["remaining"], 0);
}

/// T6, API part: a credit coupon can be scoped to the subscription it is redeemed on (CUSTOMER
/// by default); its credit then belongs to that subscription, and `wallet.funded` says so.
#[tokio::test]
async fn a_subscription_credit_coupon_funds_that_subscription_only() {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let rest = Rest::new(&env).await;
    let credit = |code: &str, scope: Option<&str>| {
        let mut discount = json!({ "type": "CREDIT", "currency": "EUR", "amount": "30.00" });
        if let Some(scope) = scope {
            discount["scope"] = json!(scope);
        }
        json!({ "code": code, "discount": discount, "reusable": false })
    };
    let (status, default) = rest.post("/api/v1/coupons", credit("ANY-SUB", None)).await;
    assert_eq!(status, StatusCode::CREATED, "{default}");
    assert_eq!(default["discount"]["scope"], "CUSTOMER");
    let (status, scoped) = rest
        .post("/api/v1/coupons", credit("THIS-SUB", Some("SUBSCRIPTION")))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{scoped}");
    assert_eq!(scoped["discount"]["scope"], "SUBSCRIPTION");
    let (_, read) = rest
        .get(&format!(
            "/api/v1/coupons/{}",
            scoped["id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(read["discount"]["scope"], "SUBSCRIPTION", "{read}");

    let coupon_id =
        common_domain::ids::CouponId::from_proto(scoped["id"].as_str().unwrap()).unwrap();
    let a = subscription()
        .coupon(coupon_id)
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let grants = format!(
        "/api/v1/customers/{}/credit-grants",
        CUST_UBER_ID.as_base62()
    );
    let (_, of_a) = rest
        .get(&format!("{grants}?subscription_id={}", a.as_base62()))
        .await;
    let coupon_grant = of_a["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|grant| grant["source"] == "COUPON")
        .cloned()
        .unwrap_or_else(|| panic!("the coupon's grant belongs to the subscription: {of_a}"));
    assert_eq!(coupon_grant["amount"], 3_000);
    assert_eq!(coupon_grant["kind"], "BONUS");
    assert_eq!(coupon_grant["subscription_id"], a.as_base62());
    let funded = funded_payload(
        &webhook_events(&env).await,
        coupon_grant["id"].as_str().unwrap(),
    );
    assert_eq!(funded["subscription_id"], a.as_base62());
}
