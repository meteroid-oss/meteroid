use meteroid_grpc::meteroid::api::customeremails::v1::customer_emails_service_server::CustomerEmailsServiceServer;
use meteroid_store::{Services, Store};

mod error;
mod mapping;
mod service;

#[derive(Clone)]
pub struct CustomerEmailsServiceComponents {
    pub store: Store,
    pub services: Services,
}

pub fn service(
    store: Store,
    services: Services,
) -> CustomerEmailsServiceServer<CustomerEmailsServiceComponents> {
    let inner = CustomerEmailsServiceComponents { store, services };
    CustomerEmailsServiceServer::new(inner)
}
