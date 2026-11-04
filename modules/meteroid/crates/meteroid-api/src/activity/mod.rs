use meteroid_grpc::meteroid::api::activity::v1::activity_service_server::ActivityServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;

pub use meteroid_grpc_mapping::activity::error;
pub mod service;

pub struct ActivityServiceComponents {
    pub store: Store,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    jwt_secret: SecretString,
) -> ActivityServiceServer<ActivityServiceComponents> {
    ActivityServiceServer::new(ActivityServiceComponents { store, jwt_secret })
}
