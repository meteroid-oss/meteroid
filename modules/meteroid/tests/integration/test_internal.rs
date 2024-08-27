use common_config::auth::InternalAuthConfig;
use common_grpc::middleware::client::auth::{create_admin_auth_layer, create_api_auth_layer};

use meteroid_grpc::meteroid::internal::v1::internal_service_client::InternalServiceClient;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

#[tokio::test]
async fn test_internal_basic() {
    // Generic setup
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;

    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );

    let created_token_response = clients
        .api_tokens
        .clone()
        .create_api_token(tonic::Request::new(
            meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenRequest {
                name: "some-api-key".to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner();

    let api_key = "pv_sand_9XzHg0EYO2Usy9ITU6bbhBnkYYbx/2vO7XtUUeQ7Wq9EZCAbBG";

    let auth_config = InternalAuthConfig {
        hmac_secret: "secret".to_string().into(),
    };

    let svc = tower::ServiceBuilder::new()
        .layer(create_admin_auth_layer(&auth_config))
        .layer(create_api_auth_layer(api_key.to_string()))
        .service(setup.channel.clone());

    let internal_client = InternalServiceClient::new(svc.clone());

    let resolved_api_key = internal_client
        .clone()
        .resolve_api_key(tonic::Request::new(
            meteroid_grpc::meteroid::internal::v1::ResolveApiKeyRequest {
                api_key_id: created_token_response.details.clone().unwrap().id.clone(),
            },
        ))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(
        resolved_api_key.tenant_id,
        created_token_response.details.clone().unwrap().tenant_id,
    );

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await;
}
