use crate::db::DbService;
use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_server::ApiTokensServiceServer;
use meteroid_repository::Pool;

mod mapping;
mod service;

pub fn service(pool: Pool) -> ApiTokensServiceServer<DbService> {
    let inner = DbService::new(pool);
    ApiTokensServiceServer::new(inner)
}
