use meteroid_grpc::meteroid::api::activity::v1::activity_service_server::ActivityServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;

mod error;
mod service;

pub struct ActivityServiceComponents {
    pub store: Store,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    jwt_secret: SecretString,
) -> ActivityServiceServer<ActivityServiceComponents> {
    let inner = ActivityServiceComponents { store, jwt_secret };
    ActivityServiceServer::new(inner)
}
