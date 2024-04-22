use std::sync::Arc;

use meteroid_grpc::meteroid::api::apitokens::v1::api_tokens_service_server::ApiTokensServiceServer;
use meteroid_store::Store;

use crate::eventbus::{Event, EventBus};

mod error;
mod mapping;
mod service;

pub struct ApiTokensServiceComponents {
    pub store: Store,
    pub eventbus: Arc<dyn EventBus<Event>>,
}

pub fn service(
    store: Store,
    eventbus: Arc<dyn EventBus<Event>>,
) -> ApiTokensServiceServer<ApiTokensServiceComponents> {
    let inner = ApiTokensServiceComponents { store, eventbus };
    ApiTokensServiceServer::new(inner)
}
