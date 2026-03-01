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
                sort_by: meteroid_grpc::meteroid::api::customers::v1::list_customer_request::SortBy::NameAsc as i32,
                pagination: None,
            },
        ))
        .await
}
