use tonic::metadata::MetadataMap;
use tonic::transport::Channel;
use tonic::{Code, Status};

use common_grpc::middleware::common::idempotency::{
    IDEMPOTENCY_CACHE_RESPONSE_HEADER, IDEMPOTENCY_KEY_HEADER,
};
use meteroid_grpc::meteroid::api::users::v1::LoginResponse;

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;
use crate::meteroid_it::svc_auth::{SEED_PASSWORD, SEED_USERNAME};

#[tokio::test]
async fn test_idempotency_cache_err_response() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    {
        // validation should fail
        let res = grpc_call_returns_err(setup.channel.clone(), Some("fake")).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code(), Code::InvalidArgument)
    }

    // ~~

    {
        // don't set idempotency key header
        // it's not required so call should pass
        let res = grpc_call_returns_err(setup.channel.clone(), None).await;
        assert!(res.is_err());
        let res = res.unwrap_err();
        let metadata = res.metadata();
        assert_eq!(res.code(), Code::PermissionDenied);
        assert!(metadata.get(IDEMPOTENCY_CACHE_RESPONSE_HEADER).is_none());
    }

    // ~~ same idempotency key
    let key = "key-0123456789";

    {
        // 1st call - set idempotency key header
        // should get original version
        let res = grpc_call_returns_err(setup.channel.clone(), Some(key)).await;
        assert!(res.is_err());
        let res = res.unwrap_err();
        let metadata = res.metadata();
        assert_eq!(res.code(), Code::PermissionDenied);
        assert_eq!(
            metadata.get(IDEMPOTENCY_CACHE_RESPONSE_HEADER).unwrap(),
            &"original"
        );
    }

    // ~~

    {
        // 2nd call - set same idempotency key header
        // should get cached version
        let res = grpc_call_returns_err(setup.channel.clone(), Some(key)).await;
        assert!(res.is_err());
        let res = res.unwrap_err();
        let metadata = res.metadata();
        assert_eq!(res.code(), Code::PermissionDenied);
        assert_eq!(
            metadata.get(IDEMPOTENCY_CACHE_RESPONSE_HEADER).unwrap(),
            &"cache"
        );
    }

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await;
}

#[tokio::test]
async fn test_idempotency_cache_ok_response() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    // ~~ same idempotency key
    let key1 = "key-1111111111";
    let key2 = "key-2222222222";
    let response_key_1_1;
    let response_key_1_2;
    let response_key_2;

    {
        // 1st call - set idempotency key header
        // should get original version
        let res = grpc_call_returns_ok(setup.channel.clone(), Some(key1)).await;
        assert!(res.is_ok());
        let (metadata, response) = res.unwrap();
        assert_eq!(
            metadata.get(IDEMPOTENCY_CACHE_RESPONSE_HEADER).unwrap(),
            &"original"
        );
        response_key_1_1 = response;
    }

    // ~~

    {
        // 2nd call - set same idempotency key header
        // should get cached version
        let res = grpc_call_returns_ok(setup.channel.clone(), Some(key1)).await;
        assert!(res.is_ok());
        let (metadata, response) = res.unwrap();
        assert_eq!(
            metadata.get(IDEMPOTENCY_CACHE_RESPONSE_HEADER).unwrap(),
            &"cache"
        );
        response_key_1_2 = response;
    }

    // ~~
    // sleep because jwt token is generated based on current time
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // ~~

    {
        // 3rd call - set NEW idempotency key header
        let res = grpc_call_returns_ok(setup.channel.clone(), Some(key2)).await;
        assert!(res.is_ok());
        let (metadata, response) = res.unwrap();
        assert_eq!(
            metadata.get(IDEMPOTENCY_CACHE_RESPONSE_HEADER).unwrap(),
            &"original"
        );
        response_key_2 = response;
    }

    // ~~

    {
        // compare responses: original vs cached vs another key
        assert_eq!(response_key_1_1, response_key_1_2);
        assert_ne!(response_key_1_1, response_key_2);
        assert_ne!(response_key_1_2, response_key_2);
    }

    // teardown
    meteroid_it::container::terminate_meteroid(setup.token, setup.join_handle).await;
}

// method always returns error because registration is not allowed
// it requires invitation key
async fn grpc_call_returns_err(
    channel: Channel,
    idempotency: Option<&str>,
) -> Result<MetadataMap, Status> {
    let mut request =
        tonic::Request::new(meteroid_grpc::meteroid::api::users::v1::RegisterRequest {
            email: "fake@user.com".to_string(),
            password: "fake-password".to_string(),
            invite_key: None,
        });

    let metadata = request.metadata_mut();

    if let Some(idempotency) = idempotency {
        let value = idempotency.parse().unwrap();
        metadata.insert(IDEMPOTENCY_KEY_HEADER, value);
    }

    AllClients::from_channel(channel, "", "")
        .users
        .clone()
        .register(request)
        .await
        .map(|r| r.metadata().to_owned())
}

async fn grpc_call_returns_ok(
    channel: Channel,
    idempotency: Option<&str>,
) -> Result<(MetadataMap, LoginResponse), Status> {
    let mut request = tonic::Request::new(meteroid_grpc::meteroid::api::users::v1::LoginRequest {
        email: SEED_USERNAME.to_string(),
        password: SEED_PASSWORD.to_string(),
    });

    let metadata = request.metadata_mut();

    if let Some(idempotency) = idempotency {
        let value = idempotency.parse().unwrap();
        metadata.insert(IDEMPOTENCY_KEY_HEADER, value);
    }

    AllClients::from_channel(channel, "", "")
        .users
        .clone()
        .login(request)
        .await
        .map(|r| (r.metadata().to_owned(), r.into_inner()))
}
