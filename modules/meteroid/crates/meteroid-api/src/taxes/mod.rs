pub mod mapping;
pub mod service;

use std::sync::Arc;

use meteroid_app::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::api::taxes::v1::taxes_service_server::TaxesServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;

pub struct TaxesServiceComponents {
    pub store: Store,
    pub object_store: Arc<dyn ObjectStoreService>,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
) -> TaxesServiceServer<TaxesServiceComponents> {
    let inner = TaxesServiceComponents {
        store,
        object_store,
        jwt_secret,
    };
    TaxesServiceServer::new(inner)
}
