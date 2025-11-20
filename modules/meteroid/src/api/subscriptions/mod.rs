use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsServiceServer;
use secrecy::SecretString;

use meteroid_store::{Services, Store};

pub mod error;
pub(crate) mod mapping;

pub use mapping::ext;

mod service;

pub struct SubscriptionServiceComponents {
    pub store: Store,
    pub services: Services,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
) -> SubscriptionsServiceServer<SubscriptionServiceComponents> {
    let inner = SubscriptionServiceComponents { store, services, jwt_secret };
    SubscriptionsServiceServer::new(inner)
}
