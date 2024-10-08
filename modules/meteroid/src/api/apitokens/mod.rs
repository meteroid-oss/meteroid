use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_server::ApiTokensServiceServer;
use meteroid_store::Store;

mod error;
mod mapping;
mod service;

pub struct ApiTokensServiceComponents {
    pub store: Store,
}

pub fn service(store: Store) -> ApiTokensServiceServer<ApiTokensServiceComponents> {
    let inner = ApiTokensServiceComponents { store };
    ApiTokensServiceServer::new(inner)
}
