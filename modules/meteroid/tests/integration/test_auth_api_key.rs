use std::str::FromStr;

use http::{HeaderName, HeaderValue};
use tonic::transport::Channel;
use tonic::{Code, Response, Status};
use tower_http::set_header::{SetRequestHeader, SetRequestHeaderLayer};

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_grpc::middleware::common::auth::API_KEY_HEADER;
use meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenResponse;
use meteroid_grpc::meteroid::api::customers::v1::ListCustomerResponse;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_client::CustomersServiceClient;
use meteroid_grpc::meteroid::api::users::v1::users_service_client::UsersServiceClient;

#[tokio::test]
async fn test_api_key() {
    // Generic setup
    helpers::init::logging();
    let postgres_connection_string = meteroid_it::container::create_test_database().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    // Try to access with fake api-key
    let svc = build_tower_svc(&setup.channel, "fake-api-key");
    let customers_svc = CustomersServiceClient::new(svc.clone());
    let customers_response = list_customers(customers_svc).await;

    assert!(customers_response.is_err());
    assert_eq!(
        customers_response.map_err(|e| e.code()).unwrap_err(),
        Code::Unauthenticated
    );

    // try to access with valid but outdated api-key
    let svc = build_tower_svc(
        &setup.channel,
        "pv_sand_5ldOh21Ipns1OpHzYbeAjvA87x3v/2vIOgNg2ElyLMxWAPn6Xz",
    );
    let customers_svc = CustomersServiceClient::new(svc.clone());
    let customers_response = list_customers(customers_svc).await;

    assert!(customers_response.is_err());
    assert_eq!(
        customers_response.map_err(|e| e.code()).unwrap_err(),
        Code::Unauthenticated
    );

    // generate API Key
    let api_token_response = generate_api_key(&setup.channel).await;

    // access with valid API Key
    let svc = build_tower_svc(&setup.channel, api_token_response.api_key.as_str());
    let customers_svc = CustomersServiceClient::new(svc.clone());

    let customers_response = list_customers(customers_svc).await;

    assert!(customers_response.is_ok());
    assert_eq!(customers_response.unwrap().into_inner().customers.len(), 0);

    // The valid call above warmed the authorization cache. A forged secret carrying the same
    // token id must still be rejected, on both transports.
    let forged = forge_same_id_key(api_token_response.api_key.as_str());
    assert_ne!(forged, api_token_response.api_key);

    let svc = build_tower_svc(&setup.channel, forged.as_str());
    let customers_svc = CustomersServiceClient::new(svc.clone());
    let customers_response = list_customers(customers_svc).await;

    assert_eq!(
        customers_response.map_err(|e| e.code()).unwrap_err(),
        Code::Unauthenticated,
        "grpc: forged secret reused the warm cache entry of a valid token id"
    );

    let rest = reqwest::Client::new();
    let customers_url = format!("{}/api/v1/customers", setup.config.rest_api_external_url);

    let ok = rest
        .get(&customers_url)
        .bearer_auth(api_token_response.api_key.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), reqwest::StatusCode::OK);

    let forged_response = rest
        .get(&customers_url)
        .bearer_auth(forged.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(
        forged_response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "rest: forged secret reused the warm cache entry of a valid token id"
    );

    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await;
}

pub(crate) async fn generate_api_key(channel: &Channel) -> CreateApiTokenResponse {
    let svc = tower::ServiceBuilder::new().service(channel.clone());
    let users_svc = UsersServiceClient::new(svc);

    let auth_token = users_svc
        .clone()
        .login(tonic::Request::new(
            meteroid_grpc::meteroid::api::users::v1::LoginRequest {
                email: meteroid_it::svc_auth::SEED_USERNAME.to_string(),
                password: meteroid_it::svc_auth::SEED_PASSWORD.to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .token;

    let clients = meteroid_it::clients::AllClients::from_channel(
        channel.clone(),
        auth_token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let tenant_response = clients
        .tenants
        .clone()
        .create_tenant(tonic::Request::new(
            meteroid_grpc::meteroid::api::tenants::v1::CreateTenantRequest {
                name: "Test Tenant".to_string(),
                environment: 0,
                disable_emails: None,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .tenant
        .unwrap();

    let clients = meteroid_it::clients::AllClients::from_channel(
        channel.clone(),
        auth_token.clone().as_str(),
        "TESTORG",
        tenant_response.slug.as_str(),
    );

    clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenRequest {
                name: "test-api-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
}

/// Rebuilds an API key with the same token id but a different secret, mimicking an attacker who
/// learned a token id and is guessing at the secret.
fn forge_same_id_key(api_key: &str) -> String {
    let (prefixed_secret, id) = api_key
        .rsplit_once('/')
        .expect("api key carries an id part");
    let (prefix, secret) = prefixed_secret
        .rsplit_once('_')
        .expect("api key carries an environment prefix");

    // Same length and alphabet as the real secret so that only its value differs.
    let forged_secret: String = secret
        .chars()
        .map(|c| if c == 'a' { 'b' } else { 'a' })
        .collect();

    format!("{prefix}_{forged_secret}/{id}")
}

fn build_tower_svc(
    channel: &Channel,
    api_key_value: &str,
) -> SetRequestHeader<Channel, HeaderValue> {
    tower::ServiceBuilder::new()
        .layer(SetRequestHeaderLayer::if_not_present(
            HeaderName::from_str(API_KEY_HEADER).unwrap(),
            HeaderValue::from_str(api_key_value).unwrap(),
        ))
        .service(channel.clone())
}

async fn list_customers(
    customers_svc: CustomersServiceClient<SetRequestHeader<Channel, HeaderValue>>,
) -> Result<Response<ListCustomerResponse>, Status> {
    customers_svc
        .clone()
        .list_customers(tonic::Request::new(
            meteroid_grpc::meteroid::api::customers::v1::ListCustomerRequest {
                search: None,
                archived: None,
                order_by: None,
                pagination: None,
            },
        ))
        .await
}
