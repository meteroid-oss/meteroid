use meteroid_grpc::meteroid::api::customers::v1::customers_service_server::CustomersServiceServer;
use meteroid_store::Store;

pub mod error;
pub mod mapping;
mod service;

pub struct CustomerServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> CustomersServiceServer<CustomerServiceComponents> {
    let inner = CustomerServiceComponents { store };
    CustomersServiceServer::new(inner)
}
