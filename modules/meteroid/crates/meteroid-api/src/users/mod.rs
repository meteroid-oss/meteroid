use std::sync::Arc;

use meteroid_app::auth_throttle::AuthThrottle;
use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::users::v1::users_service_server::UsersServiceServer;
use meteroid_store::Store;

mod error;
pub mod mapping;
mod service;

#[derive(Clone)]
pub struct UsersServiceComponents {
    store: Store,
    auth_throttle: AuthThrottle,
    feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    auth_throttle: AuthThrottle,
    feature_resolver: Arc<dyn Entitlements>,
) -> UsersServiceServer<UsersServiceComponents> {
    let inner = UsersServiceComponents {
        store,
        auth_throttle,
        feature_resolver,
    };
    UsersServiceServer::new(inner)
}
