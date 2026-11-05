//! Customer portal token: REST minting, and scope enforcement on the portal gRPC API.

use serde_json::{Value, json};
use tonic::Code;

use crate::data::ids::CUST_UBER_ID;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, PriceComponentId, SubscriptionId};
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::portal::checkout::v1::GetSlotUpgradeCheckoutRequest;
use meteroid_grpc::meteroid::portal::checkout::v1::portal_checkout_service_client::PortalCheckoutServiceClient;
use meteroid_grpc::meteroid::portal::customer::v1::GetCustomerPortalOverviewRequest;
use meteroid_grpc::meteroid::portal::customer::v1::portal_customer_service_client::PortalCustomerServiceClient;
use meteroid_grpc::meteroid::portal::shared::v1::UpdateCustomerRequest;
use meteroid_grpc::meteroid::portal::shared::v1::portal_shared_service_client::PortalSharedServiceClient;
use meteroid_store::domain::portal_tokens::is_portal_token;

fn with_portal_token<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().unwrap());
    request
}

#[tokio::test]
async fn test_rest_customer_portal_token() {
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
                name: "portal-token-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .api_key;

    let base = setup.config.rest_api_external_url.clone();
    let mint_url = format!(
        "{base}/api/v1/customers/{}/portal-token",
        CUST_UBER_ID.as_base62()
    );
    let client = http_client_tracing::client();
    let mint = |body: Option<Value>| {
        let request = client.post(&mint_url).bearer_auth(&api_key);
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        async move { request.send().await.unwrap() }
    };

    // No body: 24h token with every scope, plus the links a browser SDK needs.
    let res = mint(None).await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let minted: Value = res.json().await.unwrap();
    let token = minted["token"].as_str().unwrap().to_string();
    let portal_url = setup.config.public_url.clone();
    assert_eq!(minted["portal_url"], json!(portal_url));
    assert_eq!(
        minted["portal_link"],
        json!(format!("{portal_url}/portal/customer?token={token}"))
    );
    assert_eq!(minted["api_url"], json!(base));

    let expires_at =
        chrono::DateTime::parse_from_rfc3339(minted["expires_at"].as_str().unwrap()).unwrap();
    let ttl = expires_at.with_timezone(&chrono::Utc) - chrono::Utc::now();
    assert!(ttl > chrono::Duration::hours(23) && ttl <= chrono::Duration::hours(24));

    assert!(is_portal_token(&token), "{token}");

    // The same request hands the same token back; `manage` alone is the full set.
    let again: Value = mint(Some(json!({ "scopes": ["manage"] })))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(again["token"], json!(token));
    assert_eq!(again["expires_at"], minted["expires_at"]);

    // Read-only token, and a much shorter lifetime: both get their own token.
    let res = mint(Some(
        json!({ "scopes": ["read"], "expires_in_seconds": 3600 }),
    ))
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let read_token = res.json::<Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(read_token, token);
    let short: Value = mint(Some(json!({ "expires_in_seconds": 3600 })))
        .await
        .json()
        .await
        .unwrap();
    assert_ne!(short["token"], json!(token));
    assert_ne!(short["token"], json!(read_token));

    // Invalid scope lists are rejected.
    assert_eq!(
        mint(Some(json!({ "scopes": [] }))).await.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert!(
        mint(Some(json!({ "scopes": ["admin"] })))
            .await
            .status()
            .is_client_error()
    );

    // The portal API enforces the scopes.
    let mut portal_customer = PortalCustomerServiceClient::new(setup.channel.clone());
    let mut portal_shared = PortalSharedServiceClient::new(setup.channel.clone());

    for token in [&token, &read_token] {
        portal_customer
            .get_customer_portal_overview(with_portal_token(
                GetCustomerPortalOverviewRequest {},
                token,
            ))
            .await
            .unwrap();
    }

    let read_update = portal_shared
        .update_customer(with_portal_token(
            UpdateCustomerRequest { customer: None },
            &read_token,
        ))
        .await
        .unwrap_err();
    assert_eq!(read_update.code(), Code::PermissionDenied);

    // The slot upgrade preview is open to read tokens, so it must never book a downgrade.
    let downgrade = PortalCheckoutServiceClient::new(setup.channel.clone())
        .get_slot_upgrade_checkout(with_portal_token(
            GetSlotUpgradeCheckoutRequest {
                subscription_id: SubscriptionId::new().as_proto(),
                price_component_id: PriceComponentId::new().as_proto(),
                delta: -1,
            },
            &read_token,
        ))
        .await
        .unwrap_err();
    assert_eq!(downgrade.code(), Code::InvalidArgument);
    assert!(
        downgrade.message().contains("positive delta"),
        "{downgrade:?}"
    );

    // A manage token reaches the handler, which rejects the empty payload.
    let manage_update = portal_shared
        .update_customer(with_portal_token(
            UpdateCustomerRequest { customer: None },
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(manage_update.code(), Code::InvalidArgument);

    let garbage = portal_customer
        .get_customer_portal_overview(with_portal_token(
            GetCustomerPortalOverviewRequest {},
            "not-a-jwt",
        ))
        .await
        .unwrap_err();
    assert_eq!(garbage.code(), Code::Unauthenticated);
}
