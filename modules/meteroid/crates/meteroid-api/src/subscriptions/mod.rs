use std::sync::Arc;

use meteroid_app::features::Entitlements;
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
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
    feature_resolver: Arc<dyn Entitlements>,
) -> SubscriptionsServiceServer<SubscriptionServiceComponents> {
    let inner = SubscriptionServiceComponents {
        store,
        services,
        jwt_secret,
        feature_resolver,
    };
    SubscriptionsServiceServer::new(inner)
}
