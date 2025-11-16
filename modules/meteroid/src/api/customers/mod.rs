use crate::services::customer_ingest::CustomerIngestService;
use meteroid_grpc::meteroid::api::customers::v1::customers_ingest_service_server::CustomersIngestServiceServer;
use meteroid_grpc::meteroid::api::customers::v1::customers_service_server::CustomersServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;

pub mod error;
pub mod mapping;
mod service;
mod service_ingest;

pub struct CustomerServiceComponents {
    pub store: Store,
    pub service: Services,
    pub jwt_secret: SecretString,
    pub ingest_service: CustomerIngestService,
}

pub fn service(
    store: Store,
    service: Services,
    jwt_secret: SecretString,
    ingest_service: CustomerIngestService,
) -> CustomersServiceServer<CustomerServiceComponents> {
    let inner = CustomerServiceComponents {
        store,
        service,
        jwt_secret,
        ingest_service,
    };
    CustomersServiceServer::new(inner)
}

pub struct CustomerIngestServiceComponents {
    pub ingest_service: CustomerIngestService,
}

pub fn ingest_service(
    ingest_service: CustomerIngestService,
) -> CustomersIngestServiceServer<CustomerIngestServiceComponents> {
    let inner = CustomerIngestServiceComponents { ingest_service };
    CustomersIngestServiceServer::new(inner).max_decoding_message_size(10 * 1024 * 1024) // 10 MB
}
