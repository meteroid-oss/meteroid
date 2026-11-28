//! Restricted secret keys reach only the resources they are granted, an expired key nothing, and
//! a rolled key keeps working through its grace period while the new one works at once.
use reqwest::StatusCode;
use serde_json::json;

use common_domain::actor::Actor;
use meteroid_store::domain::api_key_permissions::{AccessLevel, ApiKeyPermissions, ApiResource};
use meteroid_store::domain::{ApiTokenNew, SecretKeyPolicy};
use meteroid_store::repositories::api_tokens::ApiTokensInterface;

use crate::data::ids::TENANT_ID;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};

async fn key(
    setup: &MeteroidSetup,
    policy: SecretKeyPolicy,
) -> (String, common_domain::ids::ApiTokenId) {
    let (key, token) = setup
        .store
        .insert_api_token_with_policy(
            Actor::System,
            ApiTokenNew {
                name: "restricted".to_owned(),
                tenant_id: TENANT_ID,
            },
            policy,
        )
        .await
        .expect("api key");
    (key, token.id)
}

async fn call(setup: &MeteroidSetup, key: &str, method: reqwest::Method, path: &str) -> StatusCode {
    let mut request = http_client_tracing::client()
        .request(
            method.clone(),
            format!("{}{path}", setup.config.rest_api_external_url),
        )
        .bearer_auth(key);
    if method == reqwest::Method::POST {
        request = request
            .header("Idempotency-Key", uuid::Uuid::now_v7().to_string())
            .json(&json!({}));
    }
    request.send().await.unwrap().status()
}

#[tokio::test]
async fn restricted_keys_expiry_and_rolling() {
    helpers::init::logging();
    let postgres = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(postgres, SeedLevel::MINIMAL).await;
    use reqwest::Method;

    // The seeded key is a row from before permissions existed: every new column NULL.
    let legacy = "pv_sand_9XzHg0EYO2Usy9ITU6bbhBnkYYbx/2vO7XtUUeQ7Wq9EZCAbBG";
    for path in [
        "/api/v1/customers",
        "/api/v2/invoices",
        "/api/v2/plans",
        "/api/v2/webhooks/endpoints",
        "/api/v2/connected-accounts",
        "/api/v2/me",
    ] {
        let status = call(&setup, legacy, Method::GET, path).await;
        assert!(
            status != StatusCode::FORBIDDEN && status != StatusCode::UNAUTHORIZED,
            "a key from before permissions keeps full access: {path} answered {status}"
        );
    }
    assert_eq!(
        call(&setup, legacy, Method::GET, "/api/v1/customers").await,
        StatusCode::OK
    );

    assert!(
        setup
            .store
            .insert_api_token_with_policy(
                Actor::System,
                ApiTokenNew {
                    name: "nothing".to_owned(),
                    tenant_id: TENANT_ID,
                },
                SecretKeyPolicy {
                    permissions: Some(ApiKeyPermissions::default()),
                    lifetime_days: None,
                },
            )
            .await
            .is_err(),
        "a restricted key granting nothing is refused"
    );

    let (read_customers, _) = key(
        &setup,
        SecretKeyPolicy {
            permissions: Some(ApiKeyPermissions::new([(
                ApiResource::Customers,
                AccessLevel::Read,
            )])),
            lifetime_days: None,
        },
    )
    .await;

    assert_eq!(
        call(&setup, &read_customers, Method::GET, "/api/v1/customers").await,
        StatusCode::OK
    );
    assert_eq!(
        call(&setup, &read_customers, Method::GET, "/api/v2/me").await,
        StatusCode::OK
    );
    for (method, path) in [
        (Method::POST, "/api/v2/customers"),
        (Method::GET, "/api/v1/invoices"),
        (Method::GET, "/api/v2/plans"),
    ] {
        assert_eq!(
            call(&setup, &read_customers, method.clone(), path).await,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }

    let (full, full_id) = key(&setup, SecretKeyPolicy::default()).await;
    assert_eq!(
        call(&setup, &full, Method::GET, "/api/v1/invoices").await,
        StatusCode::OK
    );

    let (rolled, token) = setup
        .store
        .roll_api_token(
            Actor::System,
            &full_id,
            TENANT_ID,
            chrono::Duration::hours(1),
        )
        .await
        .expect("rolled");
    assert_eq!(
        token.permissions, None,
        "a roll keeps the old key's permissions"
    );
    assert_eq!(
        call(&setup, &rolled, Method::GET, "/api/v1/invoices").await,
        StatusCode::OK
    );
    assert_eq!(
        call(&setup, &full, Method::GET, "/api/v1/invoices").await,
        StatusCode::OK,
        "the old key works through its grace period"
    );
    assert!(
        setup
            .store
            .roll_api_token(Actor::System, &full_id, TENANT_ID, chrono::Duration::zero())
            .await
            .is_err(),
        "a rolled key is rolled once"
    );

    let (to_expire, to_expire_id) = key(&setup, SecretKeyPolicy::default()).await;
    assert_eq!(
        call(&setup, &to_expire, Method::GET, "/api/v1/customers").await,
        StatusCode::OK,
        "cached before the roll"
    );
    setup
        .store
        .roll_api_token(
            Actor::System,
            &to_expire_id,
            TENANT_ID,
            chrono::Duration::zero(),
        )
        .await
        .expect("rolled");
    let mut status = StatusCode::OK;
    for _ in 0..50 {
        status = call(&setup, &to_expire, Method::GET, "/api/v1/customers").await;
        if status == StatusCode::UNAUTHORIZED {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a key rolled with no grace stops at once, cached or not"
    );
}
