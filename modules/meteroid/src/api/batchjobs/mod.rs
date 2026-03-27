use crate::services::storage::ObjectStoreService;
use meteroid_grpc::meteroid::api::batchjobs::v1::batch_jobs_service_server::BatchJobsServiceServer;
use meteroid_store::Store;
use secrecy::SecretString;
use std::sync::Arc;

mod error;
pub(crate) mod mapping;
mod service;

pub struct BatchJobsServiceComponents {
    pub store: Store,
    pub object_store: Arc<dyn ObjectStoreService>,
    pub jwt_secret: SecretString,
}

pub fn service(
    store: Store,
    object_store: Arc<dyn ObjectStoreService>,
    jwt_secret: SecretString,
) -> BatchJobsServiceServer<BatchJobsServiceComponents> {
    let inner = BatchJobsServiceComponents {
        store,
        object_store,
        jwt_secret,
    };
    BatchJobsServiceServer::new(inner).max_decoding_message_size(10 * 1024 * 1024)
}
