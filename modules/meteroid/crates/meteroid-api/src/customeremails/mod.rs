use meteroid_app::features::Entitlements;
use meteroid_grpc::meteroid::api::customeremails::v1::customer_emails_service_server::CustomerEmailsServiceServer;
use meteroid_store::{Services, Store};
use std::sync::Arc;

mod error;
mod mapping;
mod service;

#[derive(Clone)]
pub struct CustomerEmailsServiceComponents {
    pub store: Store,
    pub services: Services,
    pub feature_resolver: Arc<dyn Entitlements>,
}

pub fn service(
    store: Store,
    services: Services,
    feature_resolver: Arc<dyn Entitlements>,
) -> CustomerEmailsServiceServer<CustomerEmailsServiceComponents> {
    let inner = CustomerEmailsServiceComponents {
        store,
        services,
        feature_resolver,
    };
    CustomerEmailsServiceServer::new(inner)
}
