use meteroid_grpc::meteroid::api::customers::v1::customers_service_server::CustomersServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;

pub mod error;
pub mod mapping;
mod service;

pub struct CustomerServiceComponents {
    pub store: Store,
    pub service: Services,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    service: Services,
    jwt_secret: SecretString,
) -> CustomersServiceServer<CustomerServiceComponents> {
    let inner = CustomerServiceComponents {
        store,
        service,
        jwt_secret,
    };
    CustomersServiceServer::new(inner)
}
