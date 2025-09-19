use meteroid_grpc::meteroid::api::quotes::v1::quotes_service_server::QuotesServiceServer;
use meteroid_store::{Services, Store};
use secrecy::SecretString;

mod error;
pub mod mapping;
mod service;

pub struct QuoteServiceComponents {
    pub store: Store,
    pub services: Services,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    services: Services,
    jwt_secret: SecretString,
) -> QuotesServiceServer<QuoteServiceComponents> {
    let inner = QuoteServiceComponents {
        store,
        services,
        jwt_secret,
    };

    QuotesServiceServer::new(inner)
}
