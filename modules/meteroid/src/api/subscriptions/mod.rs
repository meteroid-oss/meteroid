use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_ingest_service_server::SubscriptionsIngestServiceServer;
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsServiceServer;
use secrecy::SecretString;

use crate::services::subscription_ingest::SubscriptionIngestService;
use meteroid_store::{Services, Store};

pub mod error;
pub(crate) mod mapping;

pub use mapping::ext;

mod service;
mod service_ingest;

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
    let inner = SubscriptionServiceComponents {
        store,
        services,
        jwt_secret,
    };
    SubscriptionsServiceServer::new(inner)
}

pub struct SubscriptionIngestServiceComponents {
    pub ingest_service: SubscriptionIngestService,
}

pub fn ingest_service(
    ingest_service: SubscriptionIngestService,
) -> SubscriptionsIngestServiceServer<SubscriptionIngestServiceComponents> {
    let inner = SubscriptionIngestServiceComponents { ingest_service };
    SubscriptionsIngestServiceServer::new(inner).max_decoding_message_size(10 * 1024 * 1024) // 10 MB
}
