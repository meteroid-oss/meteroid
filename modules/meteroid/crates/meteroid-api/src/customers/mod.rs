use std::sync::Arc;

use meteroid_grpc::meteroid::api::customers::v1::customers_service_server::CustomersServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;

use meteroid_app::features::Entitlements;

pub use meteroid_grpc_mapping::customers::{error, mapping};
mod service;

pub struct CustomerServiceComponents {
    pub store: Store,
    pub service: Services,
    pub jwt_secret: SecretString,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    service: Services,
    jwt_secret: SecretString,
    feature_resolver: Arc<dyn Entitlements>,
) -> CustomersServiceServer<CustomerServiceComponents> {
    let inner = CustomerServiceComponents {
        store,
        service,
        jwt_secret,
        feature_resolver,
    };
    CustomersServiceServer::new(inner)
}
