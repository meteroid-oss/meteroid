use tonic::Status;
use tonic::metadata::MetadataMap;
use tonic::transport::Channel;

use common_grpc::middleware::common::idempotency::{
    IDEMPOTENCY_CACHE_RESPONSE_HEADER, IDEMPOTENCY_KEY_HEADER,
};

use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;

#[tokio::test]
async fn test_idempotency() {
    helpers::init::logging();
    let (_postgres_container, postgres_connection_string) =
        meteroid_it::container::start_postgres().await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    {
        // don't set idempotency key header
        // it's not required so call should pass
        let res = grpc_call(setup.channel.clone(), None).await;
        assert!(res.is_ok());
    }

    // ~~~

    {
        // validation doesn't check because target grpc method is not using idempotency
        let res = grpc_call(setup.channel.clone(), Some("fake")).await;
        assert!(res.is_ok());
    }

    // same idempotency key

    let key = "key-0123456789";

    {
        // 1st call - set idempotency key header
        // call should pass because method doesn't use idempotency
        let res = grpc_call(setup.channel.clone(), Some(key)).await;
        assert!(res.is_ok());
        assert!(
            res.unwrap()
                .get(IDEMPOTENCY_CACHE_RESPONSE_HEADER)
                .is_none()
        );
    }

    // ~~~

    {
        // 2nd call - set same idempotency key header
        // call should pass because method doesn't use idempotency
        let res = grpc_call(setup.channel.clone(), Some(key)).await;
        assert!(res.is_ok());
        assert!(
            res.unwrap()
                .get(IDEMPOTENCY_CACHE_RESPONSE_HEADER)
                .is_none()
        );
    }
    // teardown
    // meteroid_it::container::terminate_meteroid(setup.token, &setup.join_handle).await;
}

async fn grpc_call(channel: Channel, idempotency: Option<&str>) -> Result<MetadataMap, Status> {
    let mut request =
        tonic::Request::new(meteroid_grpc::meteroid::api::instance::v1::GetInstanceRequest {});

    let metadata = request.metadata_mut();

    if let Some(idempotency) = idempotency {
        let value = idempotency.parse().unwrap();
        metadata.insert(IDEMPOTENCY_KEY_HEADER, value);
    }

    AllClients::from_channel(channel, "", "TESTORG", "")
        .instance
        .clone()
        .get_instance(request)
        .await
        .map(|r| r.metadata().to_owned())
}
